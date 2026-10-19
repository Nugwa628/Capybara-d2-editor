//! Map rendering: one `Mesh2d` per (map, layer), rebuilt when the document
//! changes. The drawing itself sits behind [`LayerBuilder`] so the textured
//! renderer can replace the placeholder fill without touching the rest.

pub mod placeholder;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::sprite_render::{ColorMaterial, MeshMaterial2d};
use d2_formats::ds1::Ds1;

use crate::doc::LayerKey;
use crate::state::AppState;

/// CPU-side mesh data for one layer. World space: y up, so iso y is negated.
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

/// The seam between the document and the pixels. The textured renderer
/// implements this too (it will also need the DT1 data, so expect a richer
/// constructor, not a different trait shape).
pub trait LayerBuilder: Send + Sync {
    fn build(&self, ds1: &Ds1, key: LayerKey) -> LayerGeometry;
}

#[derive(Resource)]
pub struct Builder(pub Box<dyn LayerBuilder>);

#[derive(Resource)]
struct LayerMaterial(Handle<ColorMaterial>);

/// Marks the entity that draws one layer of one document.
#[derive(Component)]
pub struct MapLayer {
    pub doc: u64,
    pub key: LayerKey,
    /// The document revision the mesh was built from.
    pub rev: u64,
}

/// The camera that shows the map (the egui camera is separate).
#[derive(Component)]
pub struct MapCamera;

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Builder(Box::new(placeholder::PlaceholderBuilder)))
            .add_systems(Startup, setup_material)
            .add_systems(Update, (sync_layers, sync_camera));
    }
}

fn setup_material(mut commands: Commands, mut materials: ResMut<Assets<ColorMaterial>>) {
    commands.insert_resource(LayerMaterial(
        materials.add(ColorMaterial::from(Color::WHITE)),
    ));
}

/// Create, rebuild, show/hide and remove layer entities to match the open maps.
fn sync_layers(
    mut commands: Commands,
    state: Res<AppState>,
    builder: Res<Builder>,
    material: Res<LayerMaterial>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut layers: Query<(Entity, &mut MapLayer, &mut Visibility, &mut Mesh2d)>,
) {
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
        let show = active && doc.is_visible(layer.key);
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
    transform.translation.x = view.center[0] as f32;
    transform.translation.y = -(view.center[1] as f32);
}
