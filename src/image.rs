//! Export a model to an image for display in a Discord message.

use anyhow::Result;
use three_d::Geometry;
use three_d_asset::io::Serialize;

/// Convert a model into bytes for an image.
pub async fn model_to_image(logger: &slog::Logger, gltf_file: &std::path::PathBuf) -> Result<Vec<u8>> {
    let mut cpu_model: three_d::CpuModel = three_d_asset::io::load_and_deserialize_async(&gltf_file).await?;

    let viewport = three_d::Viewport::new_at_origo(1280, 720);

    // Create a headless graphics context
    let context = three_d::HeadlessContext::new()?;

    cpu_model.geometries.iter_mut().for_each(|m| m.compute_tangents());
    let model = three_d::Model::<three_d::DeferredPhysicalMaterial>::new(&context, &cpu_model)?;

    let mut aabb = three_d::AxisAlignedBoundingBox::EMPTY;
    for m in model.iter() {
        aabb.expand_with_aabb(&m.aabb());
    }

    let size = aabb.size();
    slog::info!(logger, "size: {:?}", size);
    let min = aabb.min() + three_d::vec3(size.x * 0.1, size.y * 0.1, size.z * 0.4);
    let max = aabb.max() - three_d::vec3(size.x * 0.1, size.y * 0.3, size.z * 0.4);
    let lightbox = three_d::AxisAlignedBoundingBox::new_with_positions(&[min, max]);
    let pos = three_d::vec3(
        lightbox.min().x + 8.0 * lightbox.size().x,
        lightbox.min().y + 8.0 * lightbox.size().y,
        lightbox.min().z + 8.0 * lightbox.size().z,
    );
    slog::info!(logger, "light: {:?}", pos);
    let light = three_d::PointLight::new(
        &context,
        0.4,
        three_d::Srgba::WHITE,
        &pos,
        three_d::Attenuation::default(),
    );

    // Create a camera
    let camera_pos = three_d::vec3(
        lightbox.min().x + 8.0 * lightbox.size().x,
        lightbox.min().y + 8.0 * lightbox.size().y,
        lightbox.min().z + 8.0 * lightbox.size().z,
    );
    slog::info!(logger, "camera_pos: {:?}", camera_pos);
    let mut camera = three_d::Camera::new_orthographic(
        viewport,
        three_d::vec3(size.x * 0.5, size.y * 0.5, size.z * 0.5),
        aabb.center(),
        three_d::vec3(0.0, size.y * 5.0, 0.0),
        size.y * 5.0,
        0.1,
        1.0,
    );
    camera.rotate_around_with_fixed_up(&three_d::vec3(0.0, 0.0, 0.0), 5.0, 0.0);

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
        .clear(three_d::ClearState::color_and_depth(39.0, 245.0, 137.0, 0.63, 0.8))
        // Render the triangle with the per vertex colors defined at construction
        .render(&camera, &model, &[&light])
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
