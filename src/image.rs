//! Export a model to an image for display in a Discord message.

use anyhow::Result;

/// Convert a model into bytes for an image.
pub async fn model_to_image(_logger: &slog::Logger, client: &kittycad::Client, gltf_file: &[u8]) -> Result<Vec<u8>> {
    let ws = client
        .modeling()
        .commands_ws(None, None, None, None, Some(false))
        .await?;

    let engine = crate::engine::EngineConnection::new(ws).await?;

    // Send an import request to the engine.
    engine
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

    // Send a snapshot request to the engine.
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
        anyhow::bail!("Unexpected response from engine: {:?}", resp);
    };

    Ok(data.contents.0.to_vec())
}
