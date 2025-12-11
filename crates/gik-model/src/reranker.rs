//! Candle-based reranker model implementation.

use std::sync::Mutex;

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config as BertConfig};
use tokenizers::{PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};
use tracing::{debug, info, warn};

use crate::config::{DevicePreference, RerankerConfig};
use crate::error::{ModelError, ModelResult};
use crate::RerankerModel;

/// Maximum batch size for single inference pass.
/// Larger batches are split to avoid memory issues on GPU.
const MAX_BATCH_SIZE: usize = 8;

/// Candle-based cross-encoder reranker.
///
/// Uses a BERT model with a classifier head to score query-document pairs.
pub struct CandleRerankerModel {
    model_id: String,
    model: BertModel,
    classifier_weight: Tensor,
    classifier_bias: Tensor,
    tokenizer: Mutex<Tokenizer>,
    device: Device,
    /// Device preference for fallback logic (reserved for future use)
    #[allow(dead_code)]
    device_preference: DevicePreference,
    /// Config for potential CPU reload (reserved for future use)
    #[allow(dead_code)]
    config: RerankerConfig,
}

impl std::fmt::Debug for CandleRerankerModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CandleRerankerModel")
            .field("model_id", &self.model_id)
            .finish()
    }
}

unsafe impl Send for CandleRerankerModel {}
unsafe impl Sync for CandleRerankerModel {}

impl CandleRerankerModel {
    /// Create a new Candle reranker model.
    pub fn new(config: &RerankerConfig) -> ModelResult<Self> {
        let model_path = config.effective_model_path();

        if !model_path.exists() {
            return Err(ModelError::ModelNotFound {
                model_id: config.model_id.clone(),
                path: model_path,
            });
        }

        // Check required files
        let config_path = model_path.join("config.json");
        let weights_path = model_path.join("model.safetensors");
        let tokenizer_path = model_path.join("tokenizer.json");

        for (path, name) in [
            (&config_path, "config.json"),
            (&weights_path, "model.safetensors"),
            (&tokenizer_path, "tokenizer.json"),
        ] {
            if !path.exists() {
                return Err(ModelError::model_load(
                    &config.model_id,
                    format!("{} not found", name),
                ));
            }
        }

        info!(
            "Loading reranker model '{}' from {:?}",
            config.model_id, model_path
        );

        // Select device
        // Note: Metal GPU has known issues with batch matmul in cross-encoders.
        // We default to CPU for reliable inference. GPU support can be re-enabled
        // once Metal/Candle batch processing is fixed.
        let device = match config.device {
            DevicePreference::Auto => {
                // Default to CPU for reliability (Metal has issues)
                info!("Reranker using CPU (GPU disabled due to batch matmul issues)");
                Device::Cpu
            }
            DevicePreference::Gpu => {
                // User explicitly requested GPU - try it but warn
                if let Some(d) = Self::try_gpu() {
                    warn!("Reranker using GPU (may fail on some queries)");
                    d
                } else {
                    return Err(ModelError::DeviceNotAvailable {
                        reason: Self::gpu_not_available_reason(),
                    });
                }
            }
            DevicePreference::Cpu => Device::Cpu,
        };

        // Load config
        let bert_config: BertConfig = {
            let content = std::fs::read_to_string(&config_path)?;
            serde_json::from_str(&content)?
        };

        // Load weights
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights_path], DType::F32, &device)
                .map_err(|e| ModelError::model_load(&config.model_id, e.to_string()))?
        };

        let model = BertModel::load(vb.clone(), &bert_config)
            .map_err(|e| ModelError::model_load(&config.model_id, e.to_string()))?;

        let classifier_weight = vb
            .get((1, bert_config.hidden_size), "classifier.weight")
            .map_err(|e| {
                ModelError::model_load(&config.model_id, format!("classifier.weight: {}", e))
            })?;

        let classifier_bias = vb.get(1, "classifier.bias").map_err(|e| {
            ModelError::model_load(&config.model_id, format!("classifier.bias: {}", e))
        })?;

        // Load tokenizer
        let mut tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| ModelError::model_load(&config.model_id, e.to_string()))?;

        tokenizer.with_padding(Some(PaddingParams {
            strategy: PaddingStrategy::BatchLongest,
            pad_id: 0,
            pad_token: "[PAD]".to_string(),
            ..Default::default()
        }));

        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: 512,
                ..Default::default()
            }))
            .map_err(|e| ModelError::model_load(&config.model_id, e.to_string()))?;

        info!("Reranker model loaded successfully on {:?}", device);

        Ok(Self {
            model_id: config.model_id.clone(),
            model,
            classifier_weight,
            classifier_bias,
            tokenizer: Mutex::new(tokenizer),
            device,
            device_preference: config.device,
            config: config.clone(),
        })
    }

    /// Try to create a GPU device based on available features
    fn try_gpu() -> Option<Device> {
        // Try Metal on macOS
        #[cfg(feature = "metal")]
        {
            match Device::new_metal(0) {
                Ok(device) => {
                    return Some(device);
                }
                Err(e) => {
                    tracing::debug!("Metal not available: {}", e);
                }
            }
        }

        // Try CUDA on Windows/Linux
        #[cfg(feature = "cuda")]
        {
            match Device::new_cuda(0) {
                Ok(device) => {
                    return Some(device);
                }
                Err(e) => {
                    tracing::debug!("CUDA not available: {}", e);
                }
            }
        }

        None
    }

    /// Get reason why GPU is not available
    fn gpu_not_available_reason() -> String {
        #[cfg(all(not(feature = "metal"), not(feature = "cuda")))]
        {
            return "the candle crate has not been built with GPU support. \
                    Rebuild with --features metal (macOS) or --features cuda (NVIDIA GPU)"
                .to_string();
        }

        #[cfg(feature = "metal")]
        {
            return "Metal GPU not available on this system".to_string();
        }

        #[cfg(feature = "cuda")]
        {
            return "CUDA GPU not available. Ensure NVIDIA drivers and CUDA toolkit are installed"
                .to_string();
        }

        #[allow(unreachable_code)]
        "GPU not available".to_string()
    }

    /// Score a single batch on the model's device.
    /// Returns scores or an error if inference fails.
    fn score_batch_internal(&self, query: &str, documents: &[String]) -> ModelResult<Vec<f32>> {
        let device = &self.device;
        let tokenizer = self
            .tokenizer
            .lock()
            .map_err(|e| ModelError::reranking_failed(&self.model_id, e.to_string()))?;

        // Create query-document pairs
        let pairs: Vec<_> = documents
            .iter()
            .map(|doc| (query.to_string(), doc.clone()))
            .collect();

        let encodings = tokenizer
            .encode_batch(pairs, true)
            .map_err(|e| ModelError::tokenization(e.to_string()))?;

        drop(tokenizer); // Release lock early

        let batch_size = encodings.len();
        let max_len = encodings
            .iter()
            .map(|e| e.get_ids().len())
            .max()
            .unwrap_or(0);

        let mut input_ids = Vec::with_capacity(batch_size * max_len);
        let mut attention_mask = Vec::with_capacity(batch_size * max_len);
        let mut token_type_ids = Vec::with_capacity(batch_size * max_len);

        for encoding in &encodings {
            let ids = encoding.get_ids();
            let mask = encoding.get_attention_mask();
            let types = encoding.get_type_ids();

            input_ids.extend(ids);
            attention_mask.extend(mask);
            token_type_ids.extend(types);

            let pad = max_len - ids.len();
            input_ids.extend(std::iter::repeat_n(0u32, pad));
            attention_mask.extend(std::iter::repeat_n(0u32, pad));
            token_type_ids.extend(std::iter::repeat_n(0u32, pad));
        }

        let input_ids = Tensor::from_vec(input_ids, (batch_size, max_len), device)
            .map_err(|e| ModelError::reranking_failed(&self.model_id, e.to_string()))?;
        let attention_mask = Tensor::from_vec(attention_mask, (batch_size, max_len), device)
            .map_err(|e| ModelError::reranking_failed(&self.model_id, e.to_string()))?;
        let token_type_ids = Tensor::from_vec(token_type_ids, (batch_size, max_len), device)
            .map_err(|e| ModelError::reranking_failed(&self.model_id, e.to_string()))?;

        // Forward pass
        let hidden = self
            .model
            .forward(&input_ids, &token_type_ids, Some(&attention_mask))
            .map_err(|e| ModelError::reranking_failed(&self.model_id, e.to_string()))?;

        // Extract CLS embeddings (first token) - shape: [batch_size, hidden_size]
        // hidden is [batch_size, seq_len, hidden_size]
        let cls = hidden
            .narrow(1, 0, 1) // [batch_size, 1, hidden_size]
            .map_err(|e| ModelError::reranking_failed(&self.model_id, e.to_string()))?
            .squeeze(1) // [batch_size, hidden_size]
            .map_err(|e| ModelError::reranking_failed(&self.model_id, e.to_string()))?;

        // Apply classifier: logits = cls @ weight.T + bias
        // classifier_weight is [1, hidden_size], we need [hidden_size, 1]
        let weight_t = self
            .classifier_weight
            .t()
            .map_err(|e| ModelError::reranking_failed(&self.model_id, e.to_string()))?;

        // cls: [batch_size, hidden_size], weight_t: [hidden_size, 1]
        // result: [batch_size, 1]
        let logits = cls
            .matmul(&weight_t)
            .map_err(|e| ModelError::reranking_failed(&self.model_id, e.to_string()))?
            .broadcast_add(&self.classifier_bias)
            .map_err(|e| ModelError::reranking_failed(&self.model_id, e.to_string()))?
            .squeeze(1) // [batch_size]
            .map_err(|e| ModelError::reranking_failed(&self.model_id, e.to_string()))?;

        logits
            .to_vec1()
            .map_err(|e| ModelError::reranking_failed(&self.model_id, e.to_string()))
    }
}

impl RerankerModel for CandleRerankerModel {
    fn score_batch(&self, query: &str, documents: &[String]) -> ModelResult<Vec<f32>> {
        if documents.is_empty() {
            return Ok(vec![]);
        }

        debug!(
            "Reranking {} documents on {:?}",
            documents.len(),
            self.device
        );

        // Process in smaller batches to avoid memory issues
        let mut all_scores = Vec::with_capacity(documents.len());

        for chunk in documents.chunks(MAX_BATCH_SIZE) {
            let scores = self.score_batch_internal(query, chunk)?;
            all_scores.extend(scores);
        }

        Ok(all_scores)
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }
}
