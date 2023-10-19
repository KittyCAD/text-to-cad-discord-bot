use std::sync::Arc;

use anyhow::Result;
use expectorate::assert_contents;
use pretty_assertions::assert_eq;
use test_context::{test_context, AsyncTestContext};

struct ServerContext {
    config: crate::core::Server,
    server: dropshot::HttpServer<Arc<crate::server::context::Context>>,
    client: reqwest::Client,
}

impl ServerContext {
    pub async fn new() -> Result<Self> {
        // Find an unused port.
        let port = portpicker::pick_unused_port().ok_or_else(|| anyhow::anyhow!("no port available"))?;
        let config = crate::core::Server {
            address: format!("127.0.0.1:{}", port),
            spec_file: None,
            do_cron: false,
        };

        let env = common::types::Environment::Development;

        // Create the server in debug mode.
        let (server, _context) = crate::server::create_server(
            &config,
            env,
            &crate::Opts {
                debug: true,
                json: false,
                subcmd: crate::core::SubCommand::Server(config.clone()),
            },
        )
        .await?;

        // Sleep for 5 seconds while the server is comes up.
        std::thread::sleep(std::time::Duration::from_secs(5));

        Ok(ServerContext {
            config,
            server,
            client: reqwest::Client::new(),
        })
    }

    pub async fn stop(self) -> Result<()> {
        // Stop the server.
        self.server
            .close()
            .await
            .map_err(|e| anyhow::anyhow!("closing the server failed: {}", e))
    }

    pub fn get_url(&self, path: &str) -> String {
        format!("http://{}/{}", self.config.address, path.trim_start_matches('/'))
    }
}

#[async_trait::async_trait]
impl AsyncTestContext for ServerContext {
    async fn setup() -> Self {
        ServerContext::new().await.unwrap()
    }

    async fn teardown(self) {
        self.stop().await.unwrap();
    }
}

#[test]
fn test_openapi() {
    let mut api = crate::server::create_api_description().unwrap();
    let schema = crate::server::get_openapi(&mut api).unwrap();
    let schema_str = serde_json::to_string_pretty(&schema).unwrap();

    let spec: openapiv3::OpenAPI = serde_json::from_value(schema).expect("schema was not valid OpenAPI");

    assert_eq!(spec.openapi, "3.0.3");
    assert_eq!(spec.info.title, "KittyCAD CIO");
    assert_eq!(spec.info.version, "0.1.0");

    // Spot check a couple of items.
    assert!(!spec.paths.paths.is_empty());
    assert!(spec.paths.paths.get("/ping").is_some());

    // Check for lint errors.
    //let errors = openapi_lint::validate(&spec);
    //assert!(errors.is_empty(), "{}", errors.join("\n\n"));

    // TODO: add a tag to each endpoint.
    let tags = String::new();
    // Construct a string that helps us identify the organization of tags and
    // operations.
    /*let mut ops_by_tag = BTreeMap::<String, Vec<(String, String)>>::new();
    for (path, _, op) in spec.operations() {
        // Make sure each operation has exactly one tag. Note, we intentionally
        // do this before validating the OpenAPI output as fixing an error here
        // would necessitate refreshing the spec file again.
        /*assert_eq!(
            op.tags.len(),
            1,
            "operation '{}' has {} tags rather than 1",
            op.operation_id.as_ref().unwrap(),
            op.tags.len()
        );

        ops_by_tag
            .entry(op.tags.first().unwrap().to_string())
            .or_default()
            .push((op.operation_id.as_ref().unwrap().to_string(), path.to_string()));*/
    }

    let mut tags = String::new();
    for (tag, mut ops) in ops_by_tag {
        ops.sort();
        tags.push_str(&format!(r#"API operations found with tag "{}""#, tag));
        tags.push_str(&format!("\n{:40} {}\n", "OPERATION ID", "URL PATH"));
        for (operation_id, path) in ops {
            tags.push_str(&format!("{:40} {}\n", operation_id, path));
        }
        tags.push('\n');
    }*/

    // Confirm that the output hasn't changed. It's expected that we'll change
    // this file as the API evolves, but pay attention to the diffs to ensure
    // that the changes match your expectations.
    assert_contents("openapi/api.json", &schema_str);

    // When this fails, verify that operations on which you're adding,
    // renaming, or changing the tags are what you intend.
    assert_contents("openapi/api-tags.txt", &tags);
}

#[test_context(ServerContext)]
#[tokio::test]
#[serial_test::serial]
async fn test_root(ctx: &mut ServerContext) {
    let response = ctx.client.get(&ctx.get_url("")).send().await.unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let text = response.text().await.unwrap();
    let expected = r#""components":{""#;
    if !text.contains(expected) {
        assert_eq!(text, expected);
    }
}

#[test_context(ServerContext)]
#[tokio::test]
#[serial_test::serial]
async fn test_ping(ctx: &mut ServerContext) {
    let response = ctx.client.get(&ctx.get_url("ping")).send().await.unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), r#"{"message":"pong"}"#);
}
