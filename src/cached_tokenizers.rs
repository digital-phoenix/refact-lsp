use tokio::io::AsyncWriteExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::RwLock as StdRwLock;
use tokio::sync::RwLock as ARwLock;
use tokenizers::Tokenizer;
use reqwest::header::AUTHORIZATION;
use tracing::info;

use crate::global_context::GlobalContext;
use crate::caps::CodeAssistantCaps;


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub message: String,
    pub data: Option<serde_json::Value>,
}

async fn _download_tokenizer_file(
    http_client: &reqwest::Client,
    http_path: &str,
    api_token: String,
    to: impl AsRef<Path>,
) -> Result<(), String> {
    if to.as_ref().exists() {
        return Ok(());
    }
    info!("downloading tokenizer \"{}\" to {}...", http_path, to.as_ref().display());
    tokio::fs::create_dir_all(
            to.as_ref().parent().ok_or_else(|| "tokenizer path has no parent")?,
        )
        .await
        .map_err(|e| format!("failed to create parent dir: {}", e))?;
    let mut req = http_client.get(http_path);
    if !api_token.is_empty() {
        req = req.header(AUTHORIZATION, format!("Bearer {api_token}"))
    }
    let res = req
        .send()
        .await
        .map_err(|e| format!("failed to get response: {}", e))?
        .error_for_status()
        .map_err(|e| format!("failed to get response: {}", e))?;
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(to)
        .await
        .map_err(|e| format!("failed to open file: {}", e))?;
    file.write_all(&res.bytes().await
        .map_err(|e| format!("failed to fetch bytes: {}", e))?
    ).await.map_err(|e| format!("failed to write to file: {}", e))?;
    file.flush().await.map_err(|e| format!("failed to flush file: {}", e))?;
    Ok(())
}

pub async fn cached_tokenizer(
    caps: Arc<StdRwLock<CodeAssistantCaps>>,
    global_context: Arc<ARwLock<GlobalContext>>,
    model_name: String,
) -> Result<Arc<StdRwLock<Tokenizer>>, String> {
    let mut cx_locked = global_context.write().await;
    let client2 = cx_locked.http_client.clone();
    let cache_dir = cx_locked.cache_dir.clone();
    let tokenizer_arc = match cx_locked.tokenizer_map.get(&model_name) {
        Some(arc) => arc.clone(),
        None => {
            let tokenizer_cache_dir = std::path::PathBuf::from(cache_dir).join("tokenizers");
            tokio::fs::create_dir_all(&tokenizer_cache_dir)
                .await
                .expect("failed to create cache dir");
            let path = tokenizer_cache_dir.join(model_name.clone()).join("tokenizer.json");
            // Download it while it's locked, so another download won't start.
            let http_path;
            {
                // To avoid deadlocks, in all other places locks must be in the same order
                let caps_locked = caps.read().unwrap();
                let rewritten_model_name = caps_locked.tokenizer_rewrite_path.get(&model_name).unwrap_or(&model_name);
                http_path = caps_locked.tokenizer_path_template.replace("$MODEL", rewritten_model_name);();
            }
            _download_tokenizer_file(&client2, http_path.as_str(), cx_locked.cmdline.api_key.clone(), &path).await?;
            let tokenizer = Tokenizer::from_file(path).map_err(|e| format!("failed to load tokenizer: {}", e))?;
            let arc = Arc::new(StdRwLock::new(tokenizer));
            cx_locked.tokenizer_map.insert(model_name.clone(), arc.clone());
            arc
        }
    };
    Ok(tokenizer_arc)
}
