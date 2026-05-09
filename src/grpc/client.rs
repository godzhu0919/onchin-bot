use crate::config::Config;
use anyhow::Result;
use yellowstone_grpc_client::GeyserGrpcClient;

pub async fn create_client(
    config: &Config,
) -> Result<GeyserGrpcClient<impl yellowstone_grpc_client::Interceptor + Clone>> {
    let mut endpoint = config.grpc.endpoint.clone();

    // Ensure the endpoint has a scheme
    if !endpoint.starts_with("grpc://") && !endpoint.starts_with("grpcs://") {
        endpoint = format!("grpc://{}", endpoint);
    }

    let mut builder = GeyserGrpcClient::build_from_shared(endpoint)?;

    if let Some(token) = &config.grpc.token {
        builder = builder.x_token(Some(token.clone()))?;
    }

    let client = builder.connect().await?;

    Ok(client)
}
