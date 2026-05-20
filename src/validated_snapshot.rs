use crate::config::{Config, ProgramKind};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    path::Path,
};

pub const DEFAULT_VALIDATED_POOLS_PATH: &str = "validated_pools.snapshot";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatedPoolSnapshot {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub generated_at_unix: u64,
    #[serde(default)]
    pub pools: Vec<ValidatedPoolRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatedPoolRecord {
    pub address: String,
    pub dex_id: String,
    #[serde(default)]
    pub program_kind: Option<ProgramKind>,
    #[serde(default)]
    pub token_mint: Option<String>,
    #[serde(default)]
    pub base_mint: Option<String>,
    #[serde(default)]
    pub quote_mint: Option<String>,
    #[serde(default)]
    pub quote_liquidity_usdc: Option<f64>,
    #[serde(default)]
    pub recent_trades_5m: Option<u64>,
    #[serde(default)]
    pub recent_trades_15m: Option<u64>,
    #[serde(default)]
    pub recent_volume_5m_usd: Option<f64>,
    #[serde(default)]
    pub recent_volume_15m_usd: Option<f64>,
    #[serde(default)]
    pub first_seen_unix: Option<u64>,
    #[serde(default)]
    pub last_seen_unix: Option<u64>,
    #[serde(default)]
    pub last_seen_slot: Option<u64>,
    #[serde(default)]
    pub hits: Option<u64>,
    #[serde(default)]
    pub misses: Option<u32>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default = "default_true")]
    pub verified: bool,
}

#[derive(Debug, Clone)]
pub enum ValidatedSnapshotSource {
    File(String),
    Url(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidatedSnapshotMarker {
    File {
        modified_ns: Option<u128>,
        len: Option<u64>,
    },
    Url {
        fingerprint: Option<u64>,
    },
}

impl ValidatedPoolSnapshot {
    pub fn empty() -> Self {
        Self {
            schema_version: default_schema_version(),
            source: String::new(),
            generated_at_unix: 0,
            pools: Vec::new(),
        }
    }

    pub fn from_config(config: &Config) -> ValidatedSnapshotSource {
        let url = config.discovery.validated_pools_snapshot_url.trim();
        if !url.is_empty() {
            ValidatedSnapshotSource::Url(url.to_string())
        } else {
            ValidatedSnapshotSource::File(
                if config
                    .discovery
                    .validated_pools_snapshot_path
                    .trim()
                    .is_empty()
                {
                    DEFAULT_VALIDATED_POOLS_PATH.to_string()
                } else {
                    config.discovery.validated_pools_snapshot_path.clone()
                },
            )
        }
    }

    pub async fn load_from_config(config: &Config) -> Result<Self> {
        match Self::from_config(config) {
            ValidatedSnapshotSource::File(path) => Self::load_from_path(&path),
            ValidatedSnapshotSource::Url(url) => Self::load_from_url(&url).await,
        }
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let content = fs::read_to_string(path)
            .with_context(|| format!("read validated snapshot file {}", path.display()))?;
        Self::from_content(&content)
            .with_context(|| format!("parse validated snapshot file {}", path.display()))
    }

    pub async fn load_from_url(url: &str) -> Result<Self> {
        let response = reqwest::get(url)
            .await
            .with_context(|| format!("request validated snapshot url {}", url))?;
        if !response.status().is_success() {
            anyhow::bail!("validated snapshot HTTP status: {}", response.status());
        }
        let content = response
            .text()
            .await
            .with_context(|| format!("read validated snapshot body {}", url))?;
        Self::from_content(&content)
            .with_context(|| format!("parse validated snapshot url {}", url))
    }

    pub fn from_content(content: &str) -> Result<Self> {
        let trimmed = content.trim();
        if trimmed.is_empty() {
            return Ok(Self::empty());
        }

        if trimmed.starts_with('[') || trimmed.starts_with('{') {
            if let Ok(snapshot) = serde_json::from_str::<ValidatedPoolSnapshot>(trimmed) {
                return Ok(snapshot.normalized());
            }
            if let Ok(pools) = serde_json::from_str::<Vec<ValidatedPoolRecord>>(trimmed) {
                return Ok(Self {
                    schema_version: default_schema_version(),
                    source: String::new(),
                    generated_at_unix: 0,
                    pools,
                }
                .normalized());
            }
            if let Ok(legacy) = serde_json::from_str::<LegacyValidatedPoolSnapshot>(trimmed) {
                return Ok(legacy.into_snapshot().normalized());
            }
        }

        let mut pools = Vec::new();
        for (line_index, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let record: ValidatedPoolRecord = serde_json::from_str(line)
                .with_context(|| format!("parse validated pool jsonl line {}", line_index + 1))?;
            pools.push(record);
        }

        Ok(Self {
            schema_version: default_schema_version(),
            source: String::new(),
            generated_at_unix: 0,
            pools,
        }
        .normalized())
    }

    pub fn normalized(mut self) -> Self {
        self.pools.sort_by(|left, right| {
            left.token_mint
                .cmp(&right.token_mint)
                .then_with(|| left.dex_id.cmp(&right.dex_id))
                .then_with(|| left.address.cmp(&right.address))
        });
        self.pools
            .dedup_by(|left, right| left.address == right.address);
        self
    }

    pub fn source_fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.schema_version.hash(&mut hasher);
        self.source.hash(&mut hasher);
        self.generated_at_unix.hash(&mut hasher);
        for pool in &self.pools {
            pool.address.hash(&mut hasher);
            pool.dex_id.hash(&mut hasher);
            pool.program_kind.hash(&mut hasher);
            pool.token_mint.hash(&mut hasher);
            pool.base_mint.hash(&mut hasher);
            pool.quote_mint.hash(&mut hasher);
            pool.verified.hash(&mut hasher);
            hash_f64(pool.quote_liquidity_usdc, &mut hasher);
            pool.recent_trades_5m.hash(&mut hasher);
            pool.recent_trades_15m.hash(&mut hasher);
            hash_f64(pool.recent_volume_5m_usd, &mut hasher);
            hash_f64(pool.recent_volume_15m_usd, &mut hasher);
            pool.first_seen_unix.hash(&mut hasher);
            pool.last_seen_unix.hash(&mut hasher);
            pool.last_seen_slot.hash(&mut hasher);
            pool.hits.hash(&mut hasher);
            pool.misses.hash(&mut hasher);
        }
        hasher.finish()
    }

    pub fn pool_addresses(&self) -> Vec<String> {
        self.pools.iter().map(|pool| pool.address.clone()).collect()
    }
}

impl ValidatedSnapshotMarker {
    pub fn read(source: &ValidatedSnapshotSource) -> Self {
        match source {
            ValidatedSnapshotSource::File(path) => {
                let metadata = fs::metadata(path);
                if let Ok(metadata) = metadata {
                    Self::File {
                        modified_ns: metadata.modified().ok().and_then(system_time_to_ns),
                        len: Some(metadata.len()),
                    }
                } else {
                    Self::File {
                        modified_ns: None,
                        len: None,
                    }
                }
            }
            ValidatedSnapshotSource::Url(url) => {
                let fingerprint = reqwest::blocking::get(url)
                    .ok()
                    .and_then(|response| response.text().ok())
                    .and_then(|content| {
                        ValidatedPoolSnapshot::from_content(&content)
                            .ok()
                            .map(|snapshot| snapshot.source_fingerprint())
                    });
                Self::Url { fingerprint }
            }
        }
    }

    pub async fn refresh_changed(&mut self, source: &ValidatedSnapshotSource) -> bool {
        let current = match source {
            ValidatedSnapshotSource::File(path) => {
                let metadata = fs::metadata(path);
                if let Ok(metadata) = metadata {
                    Self::File {
                        modified_ns: metadata.modified().ok().and_then(system_time_to_ns),
                        len: Some(metadata.len()),
                    }
                } else {
                    Self::File {
                        modified_ns: None,
                        len: None,
                    }
                }
            }
            ValidatedSnapshotSource::Url(url) => {
                let fingerprint = match reqwest::get(url).await {
                    Ok(response) if response.status().is_success() => match response.text().await {
                        Ok(content) => ValidatedPoolSnapshot::from_content(&content)
                            .ok()
                            .map(|snapshot| snapshot.source_fingerprint()),
                        _ => None,
                    },
                    _ => None,
                };
                Self::Url { fingerprint }
            }
        };

        if current != *self {
            *self = current;
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyValidatedPoolSnapshot {
    #[serde(default = "default_schema_version")]
    schema_version: u32,
    #[serde(default)]
    source: String,
    #[serde(default)]
    updated_unix: u64,
    #[serde(default)]
    pools: Vec<LegacyValidatedPoolRecord>,
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyValidatedPoolRecord {
    address: String,
    #[serde(default)]
    venue: Option<String>,
    #[serde(default)]
    dex_id: Option<String>,
    #[serde(default)]
    token_mint: Option<String>,
    #[serde(default)]
    base_mint: Option<String>,
    #[serde(default)]
    quote_mint: Option<String>,
    #[serde(default)]
    quote_liquidity_usdc: Option<f64>,
    #[serde(default)]
    recent_trades_15m: Option<u64>,
    #[serde(default)]
    recent_volume_15m_usd: Option<f64>,
    #[serde(default)]
    first_seen_unix: Option<u64>,
    #[serde(default)]
    last_seen_unix: Option<u64>,
    #[serde(default)]
    last_seen_slot: Option<u64>,
    #[serde(default)]
    hits: Option<u64>,
    #[serde(default)]
    misses: Option<u32>,
    #[serde(default)]
    label: Option<String>,
}

impl LegacyValidatedPoolSnapshot {
    fn into_snapshot(self) -> ValidatedPoolSnapshot {
        ValidatedPoolSnapshot {
            schema_version: self.schema_version,
            source: self.source,
            generated_at_unix: self.updated_unix,
            pools: self
                .pools
                .into_iter()
                .map(|pool| ValidatedPoolRecord {
                    address: pool.address,
                    dex_id: pool.dex_id.or(pool.venue).unwrap_or_default(),
                    program_kind: None,
                    token_mint: pool.token_mint,
                    base_mint: pool.base_mint,
                    quote_mint: pool.quote_mint,
                    quote_liquidity_usdc: pool.quote_liquidity_usdc,
                    recent_trades_5m: None,
                    recent_trades_15m: pool.recent_trades_15m.or(pool.hits),
                    recent_volume_5m_usd: None,
                    recent_volume_15m_usd: pool.recent_volume_15m_usd,
                    first_seen_unix: pool.first_seen_unix,
                    last_seen_unix: pool.last_seen_unix,
                    last_seen_slot: pool.last_seen_slot,
                    hits: pool.hits,
                    misses: pool.misses,
                    label: pool.label,
                    verified: true,
                })
                .collect(),
        }
    }
}

fn default_schema_version() -> u32 {
    1
}

fn default_true() -> bool {
    true
}

fn system_time_to_ns(time: std::time::SystemTime) -> Option<u128> {
    time.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_nanos())
}

fn hash_f64(value: Option<f64>, hasher: &mut DefaultHasher) {
    match value {
        Some(value) if value.is_finite() => value.to_bits().hash(hasher),
        Some(_) => u64::MAX.hash(hasher),
        None => 0u64.hash(hasher),
    }
}
