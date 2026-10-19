//! The material for textured map tiles: a 2D array texture (the atlas) and a
//! premultiplied-alpha blend state.

use bevy::asset::{AssetPath, embedded_asset, embedded_path};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, BlendState, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{
    AlphaMode2d, Material2d, Material2dKey, Material2dPipeline, Material2dPlugin,
};

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct TileMaterial {
    #[texture(0, dimension = "2d_array")]
    #[sampler(1)]
    pub atlas: Handle<Image>,
}

impl Material2d for TileMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("tile_material.wesl")).with_source("embedded"),
        )
    }

    /// Blend (not opaque) puts the mesh in the transparent phase; the actual
    /// blend equation is replaced in `specialize`.
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn specialize(
        _pipeline: &Material2dPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(fragment) = descriptor.fragment.as_mut() {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING);
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
