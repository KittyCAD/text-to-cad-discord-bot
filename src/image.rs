//! Export a model to an image for display in a Discord message.

use anyhow::Result;

/// Convert a model into bytes for an image.
pub async fn model_to_image(logger: &slog::Logger, client: &kittycad::Client, gltf_file: &[u8]) -> Result<Vec<u8>> {
    let ws = client
        .modeling()
        .commands_ws(None, None, None, None, None, None, None, None, Some(false))
        .await?;

    let engine = crate::engine::EngineConnection::new(ws.0).await?;

    // Send an import request to the engine.
    slog::info!(logger, "Sending import request to engine");
    let resp = engine
        .send_modeling_cmd(
            uuid::Uuid::new_v4(),
            kittycad::types::ModelingCmd::ImportFiles {
                files: vec![kittycad::types::ImportFile {
                    path: "model.gltf".to_string(),
                    data: gltf_file.to_vec(),
                }],
                format: kittycad::types::InputFormat::Gltf {},
            },
        )
        .await?;

    let kittycad::types::OkWebSocketResponseData::Modeling {
        modeling_response: kittycad::types::OkModelingCmdResponse::ImportFiles { data },
    } = &resp
    else {
        anyhow::bail!("Unexpected response from engine import: {:?}", resp);
    };

    let object_id = data.object_id;

    // Zoom on the object.
    slog::info!(logger, "Sending zoom request to engine");
    engine
        .send_modeling_cmd(
            uuid::Uuid::new_v4(),
            kittycad::types::ModelingCmd::DefaultCameraFocusOn { uuid: object_id },
        )
        .await?;

    // Send a snapshot request to the engine.
    slog::info!(logger, "Sending snapshot request to engine");
    let resp = engine
        .send_modeling_cmd(
            uuid::Uuid::new_v4(),
            kittycad::types::ModelingCmd::TakeSnapshot {
                format: kittycad::types::ImageFormat::Png,
            },
        )
        .await?;

    let kittycad::types::OkWebSocketResponseData::Modeling {
        modeling_response: kittycad::types::OkModelingCmdResponse::TakeSnapshot { data },
    } = &resp
    else {
        anyhow::bail!("Unexpected response from engine snapshot: {:?}", resp);
    };

    Ok(data.contents.0.to_vec())
}
