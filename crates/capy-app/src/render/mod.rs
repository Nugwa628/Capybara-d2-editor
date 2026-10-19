//! Map rendering.
//!
//! * **Textured tiles and object sprites.** Per open map, the entries of
//!   `capy_render::draw_list_with_objects` in their exact order, one 2D-array
//!   atlas texture (tiles and every frame of every object layer) and a few
//!   meshes: the list is cut into runs of one blend mode, each run one mesh
//!   with strictly increasing z, so the painter's order is kept across
//!   blend states ([`scene`] has the design). The CPU work (DT1 load, draw
//!   list, sprite decode, atlas bake, quad layout) runs on the build thread
//!   ([`worker`], [`job`]); this module owns the GPU side.
//! * **Animation.** Object pieces are re-laid out on the main thread when a
//!   frame changes ("Animate objects", 25 Hz); every frame is already in the
//!   atlas, so nothing is baked or uploaded but those small meshes.
//! * **Missing tiles** get a thin magenta diamond outline.
//! * **Placeholders.** Until a map's textures are ready (and always for the
//!   Tag layer) the flat-coloured diamonds of [`placeholder`] are drawn.
//!
//! Cameras composite in gamma space (`CompositingSpace::Srgb`) so the shadow
//! blend `src + dst * (1 - a)` runs on display values like DS1Edit's.

pub mod atlas;
pub mod geometry;
pub mod job;
pub mod material;
pub mod placeholder;
pub mod scene;
pub mod worker;

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use bevy::asset::RenderAssetUsages;
use bevy::camera::CompositingSpace;
use bevy::image::ImageSampler;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};
use bevy::render::view::Msaa;
use bevy::sprite_render::{ColorMaterial, MeshMaterial2d};
use capy_render::{MissingTile, TileSet};
use d2_assets::GamePath;
use d2_formats::ds1::Ds1;

use self::atlas::AtlasLayout;
use self::geometry::{OutlineGeometry, TexGeometry};
use self::job::{BuildKey, BuildOutput};
use self::material::{BlendClass, TileMaterial, TileMaterialPlugin};
use self::scene::{Piece, Run, SceneEntry};
use self::worker::{JobRequest, JobResult, Worker};
use crate::doc::{LayerKey, MapDoc, RenderState};
use crate::state::AppState;

/// World x of the map meshes relative to iso space. `capy-render` places a
/// cell's bitmap at `(x - y) * 80`, where the cell's diamond spans
/// `[0, 160)`; iso space (the overlays, `crate::iso`) puts the diamond's top
/// corner at `(x - y) * 80`. Shifting the meshes by half a cell lines the
/// two up.
pub const WORLD_SHIFT_X: f32 = -80.0;

/// CPU-side mesh data for one placeholder layer. World space: y up, so iso y
/// is negated.
#[derive(Default)]
pub struct LayerGeometry {
    pub positions: Vec<[f32; 3]>,
    /// Linear RGBA per vertex.
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

impl LayerGeometry {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    fn into_mesh(mut self) -> Mesh {
        if self.is_empty() {
            // Zero-sized GPU buffers are not worth the risk: one degenerate triangle.
            self.positions = vec![[0.0; 3]; 3];
            self.colors = vec![[0.0; 4]; 3];
            self.indices = vec![0, 1, 2];
        }
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.colors);
        mesh.insert_indices(Indices::U32(self.indices));
        mesh
    }
}

impl From<OutlineGeometry> for LayerGeometry {
    fn from(o: OutlineGeometry) -> Self {
        LayerGeometry {
            positions: o.positions,
            colors: o.colors,
            indices: o.indices,
        }
    }
}

fn tex_mesh(geo: TexGeometry) -> Mesh {
    let TexGeometry {
        mut positions,
        mut uvs,
        mut layers,
        mut indices,
    } = geo;
    if indices.is_empty() {
        positions = vec![[0.0; 3]; 3];
        uvs = vec![[0.0; 2]; 3];
        layers = vec![[0.0; 4]; 3];
        indices = vec![0, 1, 2];
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, layers);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// The seam between the document and the pixels for the placeholder fill.
pub trait LayerBuilder: Send + Sync {
    fn build(&self, ds1: &Ds1, key: LayerKey) -> LayerGeometry;
}

#[derive(Resource)]
pub struct Builder(pub Box<dyn LayerBuilder>);

#[derive(Resource)]
struct LayerMaterial(Handle<ColorMaterial>);

/// Marks the entity that draws one placeholder layer of one document.
#[derive(Component)]
pub struct MapLayer {
    pub doc: u64,
    pub key: LayerKey,
    /// The document revision the mesh was built from.
    pub rev: u64,
}

/// One run of textured quads (tiles or sprite layers) of one document.
#[derive(Component)]
pub struct MapTiles {
    pub doc: u64,
}

/// The missing-tile outlines of one document.
#[derive(Component)]
pub struct MapOutline {
    pub doc: u64,
}

/// The camera that shows the map (the egui camera is separate).
#[derive(Component)]
pub struct MapCamera;

/// Draw depth of the outlines (opaque, so above the transparent runs, which
/// stay below [`scene::RUN_Z_MAX`]).
const OUTLINE_Z: f32 = 0.5;
/// Outline thickness in screen pixels.
const OUTLINE_PX: f32 = 2.0;
/// Added to the camera centre so sample points never sit exactly on a texel
/// edge (even zoom divisors would put them there): iso pixels.
pub const SAMPLE_BIAS: f32 = 0.02;

/// One mesh entity of a document.
struct RunSlot {
    entity: Entity,
    /// The z it was last placed at (NaN: not yet).
    z: f32,
}

/// The meshes of one piece and the object frames they show.
struct PieceRender {
    piece: Piece,
    slots: Vec<RunSlot>,
    frames: Vec<usize>,
}

/// GPU-side state of one open map.
struct DocRender {
    pieces: Vec<PieceRender>,
    outline_entity: Option<Entity>,
    /// One material per [`BlendClass`] over the current atlas.
    materials: Option<Vec<Handle<TileMaterial>>>,
    /// The tileset and atlas layout the material's image was baked from.
    tiles: Option<Arc<TileSet>>,
    layout: Option<Arc<AtlasLayout>>,
    layout_dt1: Vec<GamePath>,
    built: Option<BuildKey>,
    in_flight: Option<(u64, BuildKey)>,
    failed: Option<BuildKey>,
    missing: Vec<MissingTile>,
    bounds: Option<(i32, i32, i32, i32)>,
    outline_div: Option<u32>,
    /// The entries the meshes were made from, and the zoom they are laid out for.
    entries: Option<Arc<Vec<SceneEntry>>>,
    mesh_div: u32,
}

impl DocRender {
    fn new() -> DocRender {
        DocRender {
            pieces: Vec::new(),
            outline_entity: None,
            materials: None,
            tiles: None,
            layout: None,
            layout_dt1: Vec::new(),
            built: None,
            in_flight: None,
            failed: None,
            missing: Vec::new(),
            bounds: None,
            outline_div: None,
            entries: None,
            mesh_div: 0,
        }
    }

    fn entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.pieces
            .iter()
            .flat_map(|p| p.slots.iter().map(|s| s.entity))
            .chain(self.outline_entity)
    }
}

/// Worker-thread plumbing and per-document GPU state.
#[derive(Resource)]
pub struct MapRender {
    worker: Worker,
    rx: Mutex<Receiver<JobResult>>,
    docs: HashMap<u64, DocRender>,
    /// Parsed DT1 sets by (profile, DT1 list); maps sharing a list share one.
    tilesets: HashMap<worker::TilesetKey, Arc<TileSet>>,
    next_ticket: u64,
    /// Lay the map out for this divisor whatever the view says (the
    /// map-only screenshot hook, which may ask for any `1:N`).
    pub force_div: Option<u32>,
    /// Do not draw the missing-tile outlines (map-only screenshots, which are
    /// compared with the CPU compositor's output).
    pub hide_outlines: bool,
    /// Always show animation frame 0 (map-only screenshots).
    pub static_objects: bool,
}

impl MapRender {
    fn new() -> MapRender {
        let (tx, rx) = channel();
        MapRender {
            worker: Worker::spawn(tx),
            rx: Mutex::new(rx),
            docs: HashMap::new(),
            tilesets: HashMap::new(),
            next_ticket: 1,
            force_div: None,
            hide_outlines: false,
            static_objects: false,
        }
    }

    /// The divisor the current mesh of `doc` is laid out for, once built.
    pub fn mesh_div(&self, doc: u64) -> Option<u32> {
        self.docs
            .get(&doc)
            .filter(|d| d.built.is_some())
            .map(|d| d.mesh_div)
    }

    /// The zoom divisor the quads of `doc` should be laid out for.
    pub fn div_for(&self, doc: &MapDoc) -> u32 {
        self.force_div
            .unwrap_or_else(|| doc.view.divisor() as u32)
            .clamp(1, 16)
    }

    /// The canvas the reference would use for this map at `1:div`, in zoomed
    /// pixels `(min_x, min_y, max_x, max_y)`; see
    /// [`capy_render::reference_bounds`].
    pub fn canvas(&self, doc: &MapDoc, div: u32) -> Option<(i32, i32, i32, i32)> {
        let tiles = self.docs.get(&doc.id)?.tiles.as_ref()?;
        capy_render::reference_bounds(&doc.ds1, tiles, div)
    }
}

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(TileMaterialPlugin)
            .insert_resource(Builder(Box::new(placeholder::PlaceholderBuilder)))
            .insert_resource(MapRender::new())
            .add_systems(Startup, setup_material)
            .add_systems(
                Update,
                (
                    drive_builds,
                    sync_scene,
                    sync_layers,
                    sync_tiles_visibility,
                    sync_outlines,
                    sync_camera,
                )
                    .chain(),
            );
    }
}

fn setup_material(mut commands: Commands, mut materials: ResMut<Assets<ColorMaterial>>) {
    commands.insert_resource(LayerMaterial(
        materials.add(ColorMaterial::from(Color::WHITE)),
    ));
}

/// What the textured mesh of `doc` must be built from, once its map context is
/// known. `objects` is the session epoch of the loaded object tables.
pub fn desired_key(doc: &MapDoc, gamma: f32, objects: Option<u64>) -> Option<BuildKey> {
    let info = doc.info.as_ref()?;
    Some(BuildKey {
        rev: doc.rev,
        vis: doc.visibility(),
        gamma_bits: gamma.to_bits(),
        palette_act: info.palette_act,
        dt1_files: info.dt1_files.clone(),
        objects,
    })
}

fn atlas_image(pixels: atlas::AtlasPixels) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: pixels.size,
            height: pixels.size,
            depth_or_array_layers: pixels.layers,
        },
        TextureDimension::D2,
        pixels.data,
        // Raw display values; the camera composites in gamma space.
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    image
}

/// "f0 o0/5/2 x14" style text for the most frequent missing keys.
pub fn summarize_missing(missing: &[MissingTile], limit: usize) -> Vec<String> {
    let mut counts: HashMap<(char, u8, u8, u8, u8), usize> = HashMap::new();
    for m in missing {
        *counts
            .entry((
                m.layer.0,
                m.layer.1,
                m.key.orientation,
                m.key.main,
                m.key.sub,
            ))
            .or_default() += 1;
    }
    let mut v: Vec<_> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.into_iter()
        .take(limit)
        .map(|((kind, layer, o, main, sub), n)| {
            format!("{kind}{} orient {o} main {main} sub {sub}: {n}", layer + 1)
        })
        .collect()
}

/// Puts `runs` into the mesh entities of one piece: entities are reused in
/// order, extra ones spawned (hidden until `sync_tiles_visibility` shows
/// them) and surplus ones despawned.
fn install_runs(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &[Handle<TileMaterial>],
    doc: u64,
    slots: &mut Vec<RunSlot>,
    runs: Vec<Run>,
) {
    let n = runs.len();
    for (i, run) in runs.into_iter().enumerate() {
        let mesh = Mesh2d(meshes.add(tex_mesh(run.geo)));
        let Some(material) = materials.get(run.class.index()).cloned() else {
            continue;
        };
        match slots.get(i) {
            Some(slot) => {
                commands
                    .entity(slot.entity)
                    .insert((mesh, MeshMaterial2d(material)));
            }
            None => {
                let entity = commands
                    .spawn((
                        MapTiles { doc },
                        mesh,
                        MeshMaterial2d(material),
                        Transform::from_xyz(WORLD_SHIFT_X, 0.0, 0.0),
                        Visibility::Hidden,
                    ))
                    .id();
                slots.push(RunSlot {
                    entity,
                    z: f32::NAN,
                });
            }
        }
    }
    for slot in slots.drain(n.min(slots.len())..) {
        commands.entity(slot.entity).despawn();
    }
}

/// Gives every run of the document its z: strictly increasing in draw order.
fn assign_z(commands: &mut Commands, dr: &mut DocRender) {
    let total: usize = dr.pieces.iter().map(|p| p.slots.len()).sum();
    let mut k = 0;
    for piece in &mut dr.pieces {
        for slot in &mut piece.slots {
            let z = scene::run_z(k, total);
            if slot.z != z {
                slot.z = z;
                commands
                    .entity(slot.entity)
                    .insert(Transform::from_xyz(WORLD_SHIFT_X, 0.0, z));
            }
            k += 1;
        }
    }
}

/// The materials of an atlas image, one per blend class.
fn class_materials(
    materials: &mut Assets<TileMaterial>,
    image: &Handle<Image>,
) -> Vec<Handle<TileMaterial>> {
    BlendClass::ALL
        .iter()
        .map(|&blend| {
            materials.add(TileMaterial {
                atlas: image.clone(),
                blend,
            })
        })
        .collect()
}

/// Installs a finished build's meshes (and atlas) for document `res.doc`.
#[allow(clippy::too_many_arguments)]
fn install_build(
    commands: &mut Commands,
    dr: &mut DocRender,
    doc: u64,
    out: &mut BuildOutput,
    div: u32,
    tick: Option<u32>,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<TileMaterial>,
) {
    if let Some(pixels) = out.pixels.take() {
        let image = images.add(atlas_image(pixels));
        dr.materials = Some(class_materials(materials, &image));
    }
    let Some(class_mats) = dr.materials.clone() else {
        return;
    };
    let runs = std::mem::take(&mut out.runs);
    let mut old = std::mem::take(&mut dr.pieces);
    for (i, (piece, piece_runs)) in out.pieces.iter().zip(runs).enumerate() {
        let mut slots = old
            .get_mut(i)
            .map(|p| std::mem::take(&mut p.slots))
            .unwrap_or_default();
        install_runs(commands, meshes, &class_mats, doc, &mut slots, piece_runs);
        dr.pieces.push(PieceRender {
            frames: scene::piece_frames(&out.entries, piece, tick),
            piece: piece.clone(),
            slots,
        });
    }
    for p in old {
        for slot in p.slots {
            commands.entity(slot.entity).despawn();
        }
    }
    assign_z(commands, dr);
    dr.entries = Some(out.entries.clone());
    dr.mesh_div = div;
}

/// Starts builds that are due and installs the ones that finished.
#[allow(clippy::too_many_arguments)]
fn drive_builds(
    mut commands: Commands,
    mut state: ResMut<AppState>,
    mut render: ResMut<MapRender>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<TileMaterial>>,
    layer_material: Res<LayerMaterial>,
) {
    let gamma = state.config.gamma;
    // Edits made since the last frame (scripts run before the UI does).
    state.sync_docs();
    let catalog = state.object_catalog();
    let objects_epoch = catalog.as_ref().map(|_| state.session_epoch);

    // Drop the state of closed documents.
    let live: Vec<u64> = state.docs.iter().map(|d| d.id).collect();
    let gone: Vec<u64> = render
        .docs
        .keys()
        .copied()
        .filter(|id| !live.contains(id))
        .collect();
    for id in gone {
        if let Some(dr) = render.docs.remove(&id) {
            for e in dr.entities() {
                commands.entity(e).despawn();
            }
        }
    }

    // Finished builds.
    let results: Vec<JobResult> = match render.rx.lock() {
        Ok(rx) => rx.try_iter().collect(),
        Err(_) => Vec::new(),
    };
    for res in results {
        if let Some(t) = &res.tiles {
            render
                .tilesets
                .insert((res.profile.clone(), res.key.dt1_files.clone()), t.clone());
        }
        let Some(doc_index) = state.docs.iter().position(|d| d.id == res.doc) else {
            continue;
        };
        let wanted = desired_key(&state.docs[doc_index], gamma, objects_epoch);
        let Some(dr) = render.docs.get_mut(&res.doc) else {
            continue;
        };
        if dr.in_flight.as_ref().map(|(t, _)| *t) == Some(res.ticket) {
            dr.in_flight = None;
        }
        if wanted.as_ref() != Some(&res.key) {
            continue; // stale: the next pass starts the build that is wanted now
        }
        let path = state.docs[doc_index].path.clone();
        match res.outcome {
            Err(e) => {
                dr.failed = Some(res.key);
                state.docs[doc_index].render.state = RenderState::Failed(e.clone());
                state.error(format!("Cannot draw {path}: {e}"));
            }
            Ok(mut out) => {
                dr.tiles = res.tiles.clone().or_else(|| dr.tiles.take());
                state.docs[doc_index].tiles = dr.tiles.clone();
                dr.layout = Some(out.layout.clone());
                dr.layout_dt1 = res.key.dt1_files.clone();
                install_build(
                    &mut commands,
                    dr,
                    res.doc,
                    &mut out,
                    res.div,
                    res.tick,
                    &mut meshes,
                    &mut images,
                    &mut materials,
                );
                if dr.outline_entity.is_none() {
                    dr.outline_entity = Some(
                        commands
                            .spawn((
                                MapOutline { doc: res.doc },
                                Mesh2d(meshes.add(LayerGeometry::default().into_mesh())),
                                MeshMaterial2d(layer_material.0.clone()),
                                Transform::default(),
                                Visibility::Hidden,
                            ))
                            .id(),
                    );
                }
                dr.outline_div = None; // rebuilt by sync_outlines
                dr.bounds = out.bounds;
                dr.missing = std::mem::take(&mut out.missing);
                let objects_epoch = res.key.objects;
                dr.built = Some(res.key);
                dr.failed = None;
                let layout = out.layout.clone();
                let status = &mut state.docs[doc_index].render;
                status.state = RenderState::Ready;
                status.missing = dr.missing.len();
                status.missing_top = summarize_missing(&dr.missing, 6);
                status.tiles_drawn = out.tile_count;
                status.objects_drawn = out.object_count;
                status.objects_epoch = objects_epoch;
                status.missing_objects = out
                    .missing_objects
                    .iter()
                    .map(|m| (m.index, m.reason.to_string()))
                    .collect();
                status.object_boxes = Arc::new(scene::object_boxes(&out.entries, None));
                status.atlas = Some((layout.size, layout.layers));
                status.warnings = res.warnings.clone();
                status.millis = u64::try_from(out.elapsed.as_millis()).unwrap_or(u64::MAX);
                let message = format!(
                    "Drew {path}: {} tiles, {} object sprites{}, {} missing, atlas {}x{}x{} ({} ms)",
                    out.tile_count,
                    out.object_count,
                    if objects_epoch.is_some() {
                        ""
                    } else {
                        " (object tables not loaded yet)"
                    },
                    dr.missing.len(),
                    layout.size,
                    layout.size,
                    layout.layers,
                    out.elapsed.as_millis()
                );
                for w in &res.warnings {
                    state.warn(format!("{path}: {w}"));
                }
                state.info(message);
            }
        }
    }

    // New builds.
    let session = state
        .session
        .as_ref()
        .map(|s| (s.profile.id.clone(), s.overlay.clone()));
    let tick = shown_tick(&state, &render);
    for i in 0..state.docs.len() {
        let doc = &state.docs[i];
        let id = doc.id;
        let dr = render.docs.entry(id).or_insert_with(DocRender::new);
        let Some((profile, overlay)) = session
            .as_ref()
            .filter(|(p, _)| *p == doc.profile_id)
            .cloned()
        else {
            state.docs[i].render.state = RenderState::ProfileClosed;
            continue;
        };
        let Some(wanted) = desired_key(doc, gamma, objects_epoch) else {
            continue; // waiting for the map catalog
        };
        if dr.built.as_ref() == Some(&wanted) {
            // A context refresh that changed nothing (e.g. an override with
            // the same LvlType) marks the map Loading: it is still drawn.
            if dr.in_flight.is_none() && state.docs[i].render.state == RenderState::Loading {
                state.docs[i].render.state = RenderState::Ready;
            }
            continue;
        }
        if dr.failed.as_ref() == Some(&wanted) || dr.in_flight.is_some() {
            continue;
        }
        let tiles = render
            .tilesets
            .get(&(profile.clone(), wanted.dt1_files.clone()))
            .cloned();
        let ticket = render.next_ticket;
        render.next_ticket += 1;
        let ds1 = state.docs[i].ds1.clone();
        let div = render.div_for(&state.docs[i]);
        let dr = render.docs.entry(id).or_insert_with(DocRender::new);
        let prior = dr
            .layout
            .clone()
            .filter(|_| dr.layout_dt1 == wanted.dt1_files);
        dr.in_flight = Some((ticket, wanted.clone()));
        let doc = &mut state.docs[i];
        if doc.render.state != RenderState::Ready {
            doc.render.state = RenderState::Loading;
        }
        let catalog = wanted.objects.and(catalog.clone());
        render.worker.send(JobRequest {
            doc: id,
            ticket,
            div,
            tick,
            key: wanted,
            profile,
            overlay,
            tiles,
            ds1,
            prior,
            catalog,
        });
    }
}

/// The animation tick the map shows now.
fn shown_tick(state: &AppState, render: &MapRender) -> Option<u32> {
    shown_tick_at(state, render, state.objects.clock)
}

fn shown_tick_at(state: &AppState, render: &MapRender, seconds: f64) -> Option<u32> {
    if render.static_objects {
        return None;
    }
    crate::objects::shown_tick(
        state.config.animate_objects,
        state.objects.frame_tick,
        seconds,
    )
}

/// Re-lays the active map out when the zoom divisor changed (DS1Edit places
/// tiles on the zoomed pixel grid, see [`geometry::quad_origin`]), and the
/// object pieces whose animation frame changed.
fn sync_scene(
    mut commands: Commands,
    time: Res<Time>,
    mut state: ResMut<AppState>,
    mut render: ResMut<MapRender>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    state.objects.clock = time.elapsed_secs_f64();
    let Some(doc) = state.active_doc() else {
        return;
    };
    let div = render.div_for(doc);
    let tick = shown_tick(&state, &render);
    let id = doc.id;
    let Some(dr) = render.docs.get_mut(&id) else {
        return;
    };
    let (Some(entries), Some(layout), Some(tiles), Some(class_mats)) = (
        dr.entries.clone(),
        dr.layout.clone(),
        dr.tiles.clone(),
        dr.materials.clone(),
    ) else {
        return;
    };
    let zoom_changed = dr.mesh_div != div;
    let mut changed = false;
    for pr in &mut dr.pieces {
        if !zoom_changed && !pr.piece.objects {
            continue;
        }
        let frames = scene::piece_frames(&entries, &pr.piece, tick);
        if !zoom_changed && frames == pr.frames {
            continue;
        }
        let runs = scene::build_piece(
            &entries,
            &pr.piece,
            &layout,
            div,
            &|r| tiles.geometry(r),
            tick,
        );
        install_runs(
            &mut commands,
            &mut meshes,
            &class_mats,
            id,
            &mut pr.slots,
            runs,
        );
        pr.frames = frames;
        changed = true;
    }
    if changed {
        assign_z(&mut commands, dr);
    }
    dr.mesh_div = div;
}

/// Create, rebuild, show/hide and remove placeholder layer entities. Textured
/// maps only keep the Tag layer.
fn sync_layers(
    mut commands: Commands,
    state: Res<AppState>,
    builder: Res<Builder>,
    material: Res<LayerMaterial>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut layers: Query<(Entity, &mut MapLayer, &mut Visibility, &mut Mesh2d)>,
) {
    let wanted_key = |doc: &MapDoc, key: LayerKey| key == LayerKey::Tag || !doc.render.is_ready();
    // Existing entities: refresh or drop.
    let mut present: Vec<(u64, LayerKey)> = Vec::new();
    for (entity, mut layer, mut visibility, mut mesh2d) in &mut layers {
        let Some(doc) = state.docs.iter().find(|d| d.id == layer.doc) else {
            commands.entity(entity).despawn();
            continue;
        };
        if !doc.keys().contains(&layer.key) {
            commands.entity(entity).despawn();
            continue;
        }
        if layer.rev != doc.rev {
            let geometry = builder.0.build(&doc.ds1, layer.key);
            *mesh2d = Mesh2d(meshes.add(geometry.into_mesh()));
            layer.rev = doc.rev;
        }
        let active = state.active_doc().is_some_and(|a| a.id == doc.id);
        let show = active && doc.is_visible(layer.key) && wanted_key(doc, layer.key);
        let wanted = if show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != wanted {
            *visibility = wanted;
        }
        present.push((layer.doc, layer.key));
    }

    // New layers.
    for doc in &state.docs {
        for key in doc.keys() {
            if present.contains(&(doc.id, key)) {
                continue;
            }
            let geometry = builder.0.build(&doc.ds1, key);
            commands.spawn((
                MapLayer {
                    doc: doc.id,
                    key,
                    rev: doc.rev,
                },
                Mesh2d(meshes.add(geometry.into_mesh())),
                MeshMaterial2d(material.0.clone()),
                Transform::default(),
                Visibility::Hidden,
            ));
        }
    }
}

/// Show the textured quads of the active, ready document only.
fn sync_tiles_visibility(
    state: Res<AppState>,
    mut tiles: Query<(&MapTiles, &mut Visibility), Without<MapOutline>>,
) {
    for (tile, mut visibility) in &mut tiles {
        let show = state
            .active_doc()
            .is_some_and(|d| d.id == tile.doc && d.render.is_ready());
        let wanted = if show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

/// Keep the outline of the active document at a constant screen thickness.
fn sync_outlines(
    state: Res<AppState>,
    mut render: ResMut<MapRender>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut outlines: Query<(&MapOutline, &mut Mesh2d, &mut Visibility), Without<MapTiles>>,
) {
    for (outline, mut mesh2d, mut visibility) in &mut outlines {
        let active = state
            .active_doc()
            .filter(|d| d.id == outline.doc && d.render.is_ready() && !render.hide_outlines);
        let wanted = if active.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != wanted {
            *visibility = wanted;
        }
        let Some(doc) = active else { continue };
        let Some(dr) = render.docs.get_mut(&outline.doc) else {
            continue;
        };
        let div = doc.view.divisor() as u32;
        if dr.outline_div == Some(div) {
            continue;
        }
        dr.outline_div = Some(div);
        let geometry = geometry::build_outlines(&dr.missing, OUTLINE_PX * div as f32, OUTLINE_Z);
        *mesh2d = Mesh2d(meshes.add(LayerGeometry::from(geometry).into_mesh()));
    }
}

/// Fit the camera to the Map panel's rectangle and apply the active view.
fn sync_camera(
    state: Res<AppState>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut camera: Query<(&mut Camera, &mut Projection, &mut Transform), With<MapCamera>>,
) {
    let (Ok(window), Ok((mut camera, mut projection, mut transform))) =
        (windows.single(), camera.single_mut())
    else {
        return;
    };
    let target = UVec2::new(window.physical_width(), window.physical_height());
    let Some(rect) = state.map_rect_physical(window.scale_factor()) else {
        camera.is_active = false;
        return;
    };
    // Clamp into the window; a viewport outside the target is a GPU error.
    let mut viewport = bevy::camera::Viewport {
        physical_position: UVec2::new(rect[0].max(0.0) as u32, rect[1].max(0.0) as u32),
        physical_size: UVec2::new(rect[2].max(1.0) as u32, rect[3].max(1.0) as u32),
        ..default()
    };
    viewport.clamp_to_size(target);
    if viewport.physical_size.x == 0 || viewport.physical_size.y == 0 {
        camera.is_active = false;
        return;
    }
    camera.is_active = true;
    let (w, h) = (
        viewport.physical_size.x as f32,
        viewport.physical_size.y as f32,
    );
    camera.viewport = Some(viewport);

    let view = state.active_doc().map(|d| d.view).unwrap_or_default();
    let divisor = view.divisor() as f32;
    *projection = Projection::Orthographic(OrthographicProjection {
        scaling_mode: bevy::camera::ScalingMode::Fixed {
            width: w * divisor,
            height: h * divisor,
        },
        ..OrthographicProjection::default_2d()
    });
    transform.translation.x = view.center[0] as f32 + SAMPLE_BIAS;
    transform.translation.y = -(view.center[1] as f32) - SAMPLE_BIAS;
}

/// Components every map-rendering camera needs: gamma-space compositing (so
/// the shadow blend runs on display values) and no MSAA (quad edges must stay
/// pixel-exact).
pub fn map_camera_extras() -> (CompositingSpace, Msaa) {
    (CompositingSpace::Srgb, Msaa::Off)
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_formats::dt1::TileKey;

    #[test]
    fn missing_summary_counts_and_orders() {
        let m = |layer: (char, u8), main: u8| MissingTile {
            layer,
            x: 0,
            y: 0,
            key: TileKey {
                orientation: 0,
                main,
                sub: 1,
            },
        };
        let missing = vec![
            m(('f', 0), 3),
            m(('f', 0), 3),
            m(('w', 1), 7),
            m(('f', 0), 3),
        ];
        let summary = summarize_missing(&missing, 5);
        assert_eq!(summary.len(), 2);
        assert_eq!(summary[0], "f1 orient 0 main 3 sub 1: 3");
        assert_eq!(summary[1], "w2 orient 0 main 7 sub 1: 1");
        assert_eq!(summarize_missing(&missing, 1).len(), 1);
    }
}
