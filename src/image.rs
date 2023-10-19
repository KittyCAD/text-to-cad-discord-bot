//! Export a model to an image for display in a Discord message.

use anyhow::Result;

pub fn model_to_image() -> Result<()> {
    let viewport = three_d::Viewport::new_at_origo(1280, 720);

    // Create a headless graphics context
    let context = three_d::HeadlessContext::new().unwrap();

    // Create a camera
    let camera = three_d::Camera::new_perspective(
        viewport,
        three_d::vec3(0.0, 0.0, 2.0),
        three_d::vec3(0.0, 0.0, 0.0),
        three_d::vec3(0.0, 1.0, 0.0),
        three_d::degrees(60.0),
        0.1,
        10.0,
    );

    // Create the scene - a single colored triangle
    let mut model = three_d::Gm::new(
        three_d::Mesh::new(
            &context,
            &three_d::CpuMesh {
                positions: three_d::Positions::F32(vec![
                    three_d::vec3(0.5, -0.5, 0.0),  // bottom right
                    three_d::vec3(-0.5, -0.5, 0.0), // bottom left
                    three_d::vec3(0.0, 0.5, 0.0),   // top
                ]),
                colors: Some(vec![
                    three_d::Srgba::new(255, 0, 0, 255), // bottom right
                    three_d::Srgba::new(0, 255, 0, 255), // bottom left
                    three_d::Srgba::new(0, 0, 255, 255), // top
                ]),
                ..Default::default()
            },
        ),
        three_d::ColorMaterial::default(),
    );

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

    // Render three frames
    for frame_index in 0..3 {
        // Set the current transformation of the triangle
        model.set_transformation(three_d::Mat4::from_angle_y(three_d::radians(frame_index as f32 * 0.6)));

        // Create a render target (a combination of a color and a depth texture) to write into
        let pixels = three_d::RenderTarget::new(texture.as_color_target(None), depth_texture.as_depth_target())
            // Clear color and depth of the render target
            .clear(three_d::ClearState::color_and_depth(0.8, 0.8, 0.8, 1.0, 1.0))
            // Render the triangle with the per vertex colors defined at construction
            .render(&camera, &model, &[])
            // Read out the colors from the render target
            .read_color();

        // Save the rendered image
        use three_d_asset::io::Serialize;

        three_d_asset::io::save(
            &three_d::CpuTexture {
                data: three_d::TextureData::RgbaU8(pixels),
                width: texture.width(),
                height: texture.height(),
                ..Default::default()
            }
            .serialize(format!("headless-{}.png", frame_index))
            .unwrap(),
        )
        .unwrap();
    }

    Ok(())
}
