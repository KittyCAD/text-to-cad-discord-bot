//! Export a model to an image for display in a Discord message.

use anyhow::Result;
use three_d_asset::io::Serialize;

/// Convert a model into bytes for an image.
pub fn model_to_image(gltf_bytes: &[u8]) -> Result<Vec<u8>> {
    let mut raw_asset = three_d_asset::io::RawAssets::new();
    raw_asset.insert("model.gltf", gltf_bytes.to_vec());

    println!("Creating viewport...");
    let viewport = three_d::Viewport::new_at_origo(1280, 720);

    // Create a headless graphics context
    println!("Creating context...");
    let context = three_d::HeadlessContext::new()?;

    // Create a camera
    println!("Creating camera...");
    let camera = three_d::Camera::new_perspective(
        viewport,
        three_d::vec3(0.0, 0.0, 2.0),
        three_d::vec3(0.0, 0.0, 0.0),
        three_d::vec3(0.0, 1.0, 0.0),
        three_d::degrees(60.0),
        0.1,
        10.0,
    );

    println!("Loading model...");

    let mut cpu_model: three_d::CpuModel = raw_asset.deserialize("")?; // Empty string implies load the first.
    cpu_model.geometries.iter_mut().for_each(|m| m.compute_tangents());
    let model = three_d::Model::<three_d::PhysicalMaterial>::new(&context, &cpu_model)?;

    // Create a color texture to render into
    let mut texture = three_d::Texture2D::new_empty::<[u8; 4]>(
        &context,
        viewport.width,
        viewport.height,
        three_d::Interpolation::Nearest,
        three_d::Interpolation::Nearest,
        None,
        three_d::Wrapping::ClampToEdge,
        three_d::Wrapping::ClampToEdge,
    );

    // Also create a depth texture to support depth testing
    let mut depth_texture = three_d::DepthTexture2D::new::<f32>(
        &context,
        viewport.width,
        viewport.height,
        three_d::Wrapping::ClampToEdge,
        three_d::Wrapping::ClampToEdge,
    );

    // Create a render target (a combination of a color and a depth texture) to write into
    let pixels = three_d::RenderTarget::new(texture.as_color_target(None), depth_texture.as_depth_target())
        // Clear color and depth of the render target
        .clear(three_d::ClearState::color_and_depth(0.8, 0.8, 0.8, 1.0, 1.0))
        // Render the triangle with the per vertex colors defined at construction
        .render(&camera, &model, &[])
        // Read out the colors from the render target
        .read_color();

    // Save the rendered image
    let image_name = "output.png";
    let image_asset = &three_d::CpuTexture {
        data: three_d::TextureData::RgbaU8(pixels),
        width: texture.width(),
        height: texture.height(),
        ..Default::default()
    }
    .serialize(image_name)?;
    let image_bytes = image_asset.get(image_name)?;

    Ok(image_bytes.to_vec())
}
