//! The material for textured map tiles and object sprites: a 2D array texture
//! (the atlas) and one of four blend states, chosen by [`BlendClass`].
//!
//! Every texel in the atlas is premultiplied (see `render/atlas.rs`), so a
//! transparent texel is `(0, 0, 0, 0)` and must leave the destination alone
//! under every blend state below:
//!
//! | class | colour `src * F_src + dst * F_dst` | reference blend |
//! |---|---|---|
//! | Premultiplied | `ONE, ONE_MINUS_SRC_ALPHA` | tiles, DT1 shadows, Normal, Alpha25/50/75, Darken |
//! | Additive | `ONE, ONE` | Additive: `min(255, s + d)` |
//! | Multiply | `DST, ONE_MINUS_SRC_ALPHA` | Multiply: `s * d` |
//! | Screen | `ONE, ONE_MINUS_SRC` | `(ONE, INV_SRC_COLOR)`: `s + d * (1 - s)` |
//!
//! Alpha always uses `ONE, ONE_MINUS_SRC_ALPHA`, so a sprite drawn over the
//! empty background becomes opaque as in the CPU compositor. The target is
//! 8-bit unorm (`CompositingSpace::Srgb`), so Additive saturates like the
//! reference.

use bevy::asset::{AssetPath, embedded_asset, embedded_path};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, RenderPipelineDescriptor,
    SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{
    AlphaMode2d, Material2d, Material2dKey, Material2dPipeline, Material2dPlugin,
};

/// Which GPU blend state a run of quads needs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BlendClass {
    #[default]
    Premultiplied,
    Additive,
    Multiply,
    Screen,
}

impl BlendClass {
    pub const ALL: [BlendClass; 4] = [
        BlendClass::Premultiplied,
        BlendClass::Additive,
        BlendClass::Multiply,
        BlendClass::Screen,
    ];

    pub fn index(self) -> usize {
        match self {
            BlendClass::Premultiplied => 0,
            BlendClass::Additive => 1,
            BlendClass::Multiply => 2,
            BlendClass::Screen => 3,
        }
    }

    /// The wgpu blend state of the class (see the module docs).
    pub fn blend_state(self) -> BlendState {
        let alpha = BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::OneMinusSrcAlpha,
            operation: BlendOperation::Add,
        };
        let color = |src_factor, dst_factor| BlendComponent {
            src_factor,
            dst_factor,
            operation: BlendOperation::Add,
        };
        match self {
            BlendClass::Premultiplied => BlendState::PREMULTIPLIED_ALPHA_BLENDING,
            BlendClass::Additive => BlendState {
                color: color(BlendFactor::One, BlendFactor::One),
                alpha,
            },
            BlendClass::Multiply => BlendState {
                color: color(BlendFactor::Dst, BlendFactor::OneMinusSrcAlpha),
                alpha,
            },
            BlendClass::Screen => BlendState {
                color: color(BlendFactor::One, BlendFactor::OneMinusSrc),
                alpha,
            },
        }
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(TileMaterialKey)]
pub struct TileMaterial {
    #[texture(0, dimension = "2d_array")]
    #[sampler(1)]
    pub atlas: Handle<Image>,
    pub blend: BlendClass,
}

/// The pipeline key: one pipeline per blend class.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileMaterialKey {
    blend: u32,
}

impl From<&TileMaterial> for TileMaterialKey {
    fn from(material: &TileMaterial) -> Self {
        TileMaterialKey {
            blend: material.blend.index() as u32,
        }
    }
}

impl Material2d for TileMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("tile_material.wesl")).with_source("embedded"),
        )
    }

    /// Blend (not opaque) puts the mesh in the transparent phase, which draws
    /// meshes by increasing z; the actual blend equation is replaced in
    /// `specialize`.
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn specialize(
        _pipeline: &Material2dPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let class = BlendClass::ALL
            .get(key.bind_group_data.blend as usize)
            .copied()
            .unwrap_or_default();
        if let Some(fragment) = descriptor.fragment.as_mut() {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(class.blend_state());
            }
        }
        Ok(())
    }
}

pub struct TileMaterialPlugin;

impl Plugin for TileMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "tile_material.wesl");
        app.add_plugins(Material2dPlugin::<TileMaterial>::default());
    }
}
