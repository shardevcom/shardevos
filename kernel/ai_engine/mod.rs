//! AI Engine — Pipeline Multimodal de Inteligencia Artificial de shardevos.
//!
//! Integración de modelos de IA de ejecución local de ultra-baja latencia:
//! - Voz a Texto (STT): whisper.cpp / Sherpa-ONNX
//! - Texto a Voz (TTS): Piper TTS
//! - SLM local con Function Calling sobre APIs del sistema operativo


pub mod stt;
pub mod tts;
pub mod slm;
pub mod embeddings;

pub use self::embeddings::{embed_text, init_embedding_model, text_similarity, Embedding, EMBEDDING_DIM};

use core::sync::atomic::{AtomicU64, Ordering};

/// Tipo de modelo de IA soportado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AiModelType {
    STT = 0,
    TTS = 1,
    SLM = 2,
}

/// Estado de un modelo de IA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AiModelState {
    Unloaded = 0,
    Loading = 1,
    Ready = 2,
    Error = 3,
    Inferring = 4,
}

/// Resultado de una operación de IA.
pub type AiResult<T> = Result<T, AiError>;

/// Errores del motor de IA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiError {
    ModelNotLoaded,
    InferenceFailed,
    InvalidInput,
    OutOfMemory,
    DeviceNotFound,
    NotInitialized,
    FunctionCallFailed,
}

/// Tamaño máximo del buffer de datos de entrada/salida de IA.
const AI_MAX_DATA: usize = 4096;

/// Buffer de datos de IA (reemplaza heapless::Vec para evitar dependencias).
#[derive(Debug, Clone)]
pub struct AiDataBuffer {
    data: [u8; AI_MAX_DATA],
    len: usize,
}

impl AiDataBuffer {
    pub const fn new() -> Self {
        Self { data: [0; AI_MAX_DATA], len: 0 }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.data[..self.len]
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.data[..self.len]
    }

    pub fn push_slice(&mut self, src: &[u8]) -> Result<(), AiError> {
        if self.len + src.len() > AI_MAX_DATA {
            return Err(AiError::OutOfMemory);
        }
        self.data[self.len..self.len + src.len()].copy_from_slice(src);
        self.len += src.len();
        Ok(())
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }
}

impl Default for AiDataBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Solicitud de inferencia genérica.
#[derive(Debug, Clone)]
#[repr(C)]
pub struct AiInferenceRequest {
    pub model_type: AiModelType,
    pub input_data: AiDataBuffer,
    pub timestamp_us: u64,
    pub request_id: u64,
}

/// Respuesta de inferencia genérica.
#[derive(Debug, Clone)]
#[repr(C)]
pub struct AiInferenceResponse {
    pub request_id: u64,
    pub model_type: AiModelType,
    pub output_data: AiDataBuffer,
    pub latency_us: u64,
    pub success: bool,
}

/// Trait para modelos de IA.
pub trait AiModel {
    fn model_type(&self) -> AiModelType;
    fn state(&self) -> AiModelState;
    fn load(&mut self) -> AiResult<()>;
    fn unload(&mut self) -> AiResult<()>;
    fn infer(&mut self, request: &AiInferenceRequest) -> AiResult<AiInferenceResponse>;
}

/// Contador global de solicitudes de IA.
static AI_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

/// Genera un ID único de solicitud de IA.
pub(crate) fn next_request_id() -> u64 {
    AI_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
}

/// Devuelve el tiempo actual en microsegundos (placeholder, usar TSC/RTC en producción).
pub(crate) fn time_us() -> u64 {
    // Placeholder: contador simple basado en request_id
    next_request_id() * 1000
}
static AI_ENGINE: AtomicU64 = AtomicU64::new(0);

/// Devuelve true si el motor de IA está inicializado.
pub fn is_initialized() -> bool {
    AI_ENGINE.load(Ordering::Acquire) != 0
}

/// Inicializa el motor de IA.
pub unsafe fn init() {
    AI_ENGINE.store(1, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_model_types() {
        assert_eq!(AiModelType::STT as u8, 0);
        assert_eq!(AiModelType::TTS as u8, 1);
        assert_eq!(AiModelType::SLM as u8, 2);
    }

    #[test]
    fn ai_model_states() {
        assert_eq!(AiModelState::Unloaded as u8, 0);
        assert_eq!(AiModelState::Ready as u8, 2);
        assert_eq!(AiModelState::Error as u8, 3);
    }

    #[test]
    fn next_request_id_increments() {
        let id1 = next_request_id();
        let id2 = next_request_id();
        assert_eq!(id2, id1 + 1);
    }

    #[test]
    fn ai_error_codes() {
        assert_eq!(AiError::ModelNotLoaded as isize, 0);
        assert_eq!(AiError::FunctionCallFailed as isize, 6);
    }

    #[test]
    fn ai_data_buffer_push() {
        let mut buf = AiDataBuffer::new();
        assert!(buf.push_slice(b"hello").is_ok());
        assert_eq!(buf.len(), 5);
        assert_eq!(buf.as_slice(), b"hello");

        // Llenar el buffer completamente
        let big = [0xAAu8; AI_MAX_DATA];
        buf.clear();
        assert!(buf.push_slice(&big).is_ok());
        assert_eq!(buf.len(), AI_MAX_DATA);

        // Intentar exceder
        assert!(buf.push_slice(b"x").is_err());
    }

    #[test]
    fn ai_data_buffer_clear() {
        let mut buf = AiDataBuffer::new();
        buf.push_slice(b"test").unwrap();
        buf.clear();
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());
    }
}
