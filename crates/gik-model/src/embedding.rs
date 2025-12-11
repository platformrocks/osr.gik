//! Candle-based embedding model implementation.

use std::path::Path;
use std::sync::Mutex;

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config as BertConfig, DTYPE};
use candle_transformers::models::xlm_roberta::{Config as XLMRobertaConfig, XLMRobertaModel};
use tokenizers::{PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};
use tracing::info;

use crate::config::{
    DevicePreference, EmbeddingConfig, HuggingFaceModelConfig, ModelArchitecture, ModelInfo,
};
use crate::error::{ModelError, ModelResult};
use crate::EmbeddingModel;

// ============================================================================
// ModelBackend enum
// ============================================================================

enum ModelBackend {
    Bert(BertModel),
    Roberta(XLMRobertaModel),
}

impl ModelBackend {
    fn forward(
        &self,
        input_ids: &Tensor,
        token_type_ids: &Tensor,
        attention_mask: &Tensor,
    ) -> ModelResult<Tensor> {
        match self {
            ModelBackend::Bert(model) => model
                .forward(input_ids, token_type_ids, Some(attention_mask))
                .map_err(|e| {
                    ModelError::embedding_failed("bert", format!("Forward failed: {}", e))
                }),
            ModelBackend::Roberta(model) => model
                .forward(input_ids, attention_mask, token_type_ids, None, None, None)
                .map_err(|e| {
                    ModelError::embedding_failed("roberta", format!("Forward failed: {}", e))
                }),
        }
    }
}

// ============================================================================
// CandleEmbeddingModel
// ============================================================================

/// Candle-based embedding model.
///
/// Supports BERT and RoBERTa architectures with mean pooling and L2 normalization.
pub struct CandleEmbeddingModel {
    model_info: ModelInfo,
    model: Mutex<ModelBackend>,
    tokenizer: Mutex<Tokenizer>,
    device: Device,
}

impl std::fmt::Debug for CandleEmbeddingModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CandleEmbeddingModel")
            .field("model_id", &self.model_info.model_id)
            .field("dimension", &self.model_info.dimension)
            .finish()
    }
}

unsafe impl Send for CandleEmbeddingModel {}
unsafe impl Sync for CandleEmbeddingModel {}

impl CandleEmbeddingModel {
    /// Create a new Candle embedding model.
    pub fn new(config: &EmbeddingConfig) -> ModelResult<Self> {
        let model_path = config.effective_model_path();

        if !model_path.exists() {
            return Err(ModelError::ModelNotFound {
                model_id: config.model_id.clone(),
                path: model_path,
            });
        }

        // Load HuggingFace config
        let hf_config = Self::load_hf_config(&model_path)?;
        let architecture = hf_config.infer_architecture();
        let dimension = hf_config.hidden_size;
        let max_seq_len = config
            .max_sequence_length
            .min(hf_config.max_position_embeddings);

        info!(
            "Loading embedding model '{}' from {:?} (arch={}, dim={})",
            config.model_id, model_path, architecture, dimension
        );

        // Tokenizer config based on architecture
        let (pad_id, pad_token) = match architecture {
            ModelArchitecture::Bert => (0, "[PAD]"),
            ModelArchitecture::Roberta | ModelArchitecture::Mpnet => (1, "<pad>"),
            ModelArchitecture::Unknown => (0, "[PAD]"),
        };

        let tokenizer = Self::load_tokenizer(&model_path, max_seq_len, pad_id, pad_token)?;
        let device = Self::select_device(config.device)?;
        let model = Self::load_model(&model_path, architecture, &device)?;

        let model_info = ModelInfo::new(&config.model_id, dimension, max_seq_len)
            .with_architecture(architecture);

        Ok(Self {
            model_info,
            model: Mutex::new(model),
            tokenizer: Mutex::new(tokenizer),
            device,
        })
    }

    fn load_hf_config(model_path: &Path) -> ModelResult<HuggingFaceModelConfig> {
        let config_path = model_path.join("config.json");
        if !config_path.exists() {
            return Err(ModelError::model_load(
                model_path.display().to_string(),
                "config.json not found",
            ));
        }
        let content = std::fs::read_to_string(&config_path)?;
        Ok(serde_json::from_str(&content)?)
    }

    fn load_tokenizer(
        model_path: &Path,
        max_length: usize,
        pad_id: u32,
        pad_token: &str,
    ) -> ModelResult<Tokenizer> {
        let tokenizer_path = model_path.join("tokenizer.json");
        if !tokenizer_path.exists() {
            return Err(ModelError::model_load(
                model_path.display().to_string(),
                "tokenizer.json not found",
            ));
        }

        let mut tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| ModelError::model_load(model_path.display().to_string(), e.to_string()))?;

        tokenizer.with_padding(Some(PaddingParams {
            strategy: PaddingStrategy::BatchLongest,
            pad_id,
            pad_token: pad_token.to_string(),
            ..Default::default()
        }));

        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length,
                ..Default::default()
            }))
            .map_err(|e| ModelError::model_load(model_path.display().to_string(), e.to_string()))?;

        Ok(tokenizer)
    }

    fn select_device(pref: DevicePreference) -> ModelResult<Device> {
        match pref {
            DevicePreference::Auto => {
                // Try GPU first, fall back to CPU
                if let Some(device) = Self::try_gpu() {
                    Ok(device)
                } else {
                    info!("Using CPU");
                    Ok(Device::Cpu)
                }
            }
            DevicePreference::Gpu => Self::try_gpu().ok_or_else(|| ModelError::DeviceNotAvailable {
                reason: Self::gpu_not_available_reason(),
            }),
            DevicePreference::Cpu => Ok(Device::Cpu),
        }
    }

    /// Try to create a GPU device based on available features
    fn try_gpu() -> Option<Device> {
        // Try Metal on macOS
        #[cfg(feature = "metal")]
        {
            match Device::new_metal(0) {
                Ok(device) => {
                    info!("Using Metal GPU");
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
                    info!("Using CUDA GPU");
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

    fn load_model(
        model_path: &Path,
        architecture: ModelArchitecture,
        device: &Device,
    ) -> ModelResult<ModelBackend> {
        let weights_path = model_path.join("model.safetensors");
        if !weights_path.exists() {
            return Err(ModelError::model_load(
                model_path.display().to_string(),
                "model.safetensors not found",
            ));
        }

        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights_path], DTYPE, device).map_err(|e| {
                ModelError::model_load(model_path.display().to_string(), e.to_string())
            })?
        };

        match architecture {
            ModelArchitecture::Bert | ModelArchitecture::Unknown => {
                let config_path = model_path.join("config.json");
                let content = std::fs::read_to_string(&config_path)?;
                let bert_config: BertConfig = serde_json::from_str(&content)?;
                let model = BertModel::load(vb, &bert_config).map_err(|e| {
                    ModelError::model_load(model_path.display().to_string(), e.to_string())
                })?;
                Ok(ModelBackend::Bert(model))
            }
            ModelArchitecture::Roberta | ModelArchitecture::Mpnet => {
                let config_path = model_path.join("config.json");
                let content = std::fs::read_to_string(&config_path)?;
                let roberta_config: XLMRobertaConfig = serde_json::from_str(&content)?;
                let model = XLMRobertaModel::new(&roberta_config, vb).map_err(|e| {
                    ModelError::model_load(model_path.display().to_string(), e.to_string())
                })?;
                Ok(ModelBackend::Roberta(model))
            }
        }
    }

    fn mean_pooling(&self, embeddings: &Tensor, mask: &Tensor) -> ModelResult<Tensor> {
        let mask_expanded = mask
            .unsqueeze(2)
            .map_err(|e| ModelError::embedding_failed(&self.model_info.model_id, e.to_string()))?
            .to_dtype(DType::F32)
            .map_err(|e| ModelError::embedding_failed(&self.model_info.model_id, e.to_string()))?
            .broadcast_as(embeddings.shape())
            .map_err(|e| ModelError::embedding_failed(&self.model_info.model_id, e.to_string()))?;

        let sum = embeddings
            .broadcast_mul(&mask_expanded)
            .map_err(|e| ModelError::embedding_failed(&self.model_info.model_id, e.to_string()))?
            .sum(1)
            .map_err(|e| ModelError::embedding_failed(&self.model_info.model_id, e.to_string()))?;

        let count = mask_expanded
            .sum(1)
            .map_err(|e| ModelError::embedding_failed(&self.model_info.model_id, e.to_string()))?
            .clamp(1e-9, f64::MAX)
            .map_err(|e| ModelError::embedding_failed(&self.model_info.model_id, e.to_string()))?;

        sum.broadcast_div(&count)
            .map_err(|e| ModelError::embedding_failed(&self.model_info.model_id, e.to_string()))
    }

    fn l2_normalize(&self, embeddings: &Tensor) -> ModelResult<Tensor> {
        let norm = embeddings
            .sqr()
            .map_err(|e| ModelError::embedding_failed(&self.model_info.model_id, e.to_string()))?
            .sum_keepdim(1)
            .map_err(|e| ModelError::embedding_failed(&self.model_info.model_id, e.to_string()))?
            .sqrt()
            .map_err(|e| ModelError::embedding_failed(&self.model_info.model_id, e.to_string()))?
            .clamp(1e-12, f64::MAX)
            .map_err(|e| ModelError::embedding_failed(&self.model_info.model_id, e.to_string()))?;

        embeddings
            .broadcast_div(&norm)
            .map_err(|e| ModelError::embedding_failed(&self.model_info.model_id, e.to_string()))
    }
}

impl EmbeddingModel for CandleEmbeddingModel {
    fn embed(&self, texts: &[&str]) -> ModelResult<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        let tokenizer = self
            .tokenizer
            .lock()
            .map_err(|e| ModelError::embedding_failed(&self.model_info.model_id, e.to_string()))?;

        let inputs: Vec<String> = texts.iter().map(|s| s.to_string()).collect();
        let encodings = tokenizer
            .encode_batch(inputs, true)
            .map_err(|e| ModelError::tokenization(e.to_string()))?;

        let batch_size = encodings.len();
        let seq_len = encodings.first().map(|e| e.get_ids().len()).unwrap_or(0);

        let token_ids: Vec<u32> = encodings
            .iter()
            .flat_map(|e| e.get_ids().to_vec())
            .collect();
        let attention_mask: Vec<u32> = encodings
            .iter()
            .flat_map(|e| e.get_attention_mask().to_vec())
            .collect();

        let token_ids_tensor = Tensor::from_vec(token_ids, (batch_size, seq_len), &self.device)
            .map_err(|e| ModelError::embedding_failed(&self.model_info.model_id, e.to_string()))?;
        let attention_mask_tensor =
            Tensor::from_vec(attention_mask, (batch_size, seq_len), &self.device).map_err(|e| {
                ModelError::embedding_failed(&self.model_info.model_id, e.to_string())
            })?;
        let token_type_ids = token_ids_tensor
            .zeros_like()
            .map_err(|e| ModelError::embedding_failed(&self.model_info.model_id, e.to_string()))?;

        let model = self
            .model
            .lock()
            .map_err(|e| ModelError::embedding_failed(&self.model_info.model_id, e.to_string()))?;
        let hidden_states =
            model.forward(&token_ids_tensor, &token_type_ids, &attention_mask_tensor)?;
        drop(model);

        let pooled = self.mean_pooling(&hidden_states, &attention_mask_tensor)?;
        let normalized = self.l2_normalize(&pooled)?;

        let data: Vec<f32> = normalized
            .to_vec2::<f32>()
            .map_err(|e| ModelError::embedding_failed(&self.model_info.model_id, e.to_string()))?
            .into_iter()
            .flatten()
            .collect();

        let dim = self.model_info.dimension;
        Ok(data.chunks(dim).map(|c| c.to_vec()).collect())
    }

    fn dimension(&self) -> usize {
        self.model_info.dimension
    }

    fn max_sequence_length(&self) -> usize {
        self.model_info.max_seq_len
    }

    fn model_info(&self) -> &ModelInfo {
        &self.model_info
    }
}
