//! STT (Speech-to-Text) — Reconocimiento de Voz a Texto.
//!
//! Integración con modelos de ejecución local de ultra-baja latencia:
//! whisper.cpp / Sherpa-ONNX procesados vía GPU/NPU en espacio de usuario.

use super::{AiModel, AiModelType, AiModelState, AiResult, AiError, AiInferenceRequest, AiInferenceResponse, AiDataBuffer, time_us};
use core::sync::atomic::{AtomicU64, Ordering};

/// Configuración del modelo STT.
#[derive(Debug, Clone, Copy)]
pub struct SttConfig {
    pub sample_rate: u32,
    pub language: [u8; 8],
    pub beam_size: u8,
    pub use_gpu: bool,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16000,
            language: [101, 115, 45, 69, 83, 0, 0, 0], // "es-ES\0\0"
            beam_size: 5,
            use_gpu: true,
        }
    }
}

/// Resultado de transcripción STT.
#[derive(Debug, Clone)]
pub struct SttTranscription {
    pub text: TextBuffer,
    pub confidence: f32,
    pub language_detected: [u8; 8],
    pub duration_ms: u32,
}

/// Buffer de texto simple para STT (sin dependencia de heapless).
#[derive(Debug, Clone)]
pub struct TextBuffer {
    data: [u8; 512],
    len: usize,
}

impl TextBuffer {
    pub const fn new() -> Self {
        Self { data: [0; 512], len: 0 }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.data[..self.len]).unwrap_or("")
    }

    pub fn set_str(&mut self, s: &str) -> Result<(), AiError> {
        if s.len() > 512 {
            return Err(AiError::OutOfMemory);
        }
        self.data[..s.len()].copy_from_slice(s.as_bytes());
        self.len = s.len();
        Ok(())
    }

    pub fn from_static(s: &'static str) -> Self {
        let mut tb = Self::new();
        let _ = tb.set_str(s);
        tb
    }
}

impl Default for TextBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Motor STT (whisper.cpp / Sherpa-ONNX).
pub struct SttEngine {
    config: SttConfig,
    state: AtomicU64,
    model_loaded: AtomicU64,
    inference_count: AtomicU64,
    total_latency_us: AtomicU64,
}

impl SttEngine {
    pub fn new() -> Self {
        Self {
            config: SttConfig::default(),
            state: AtomicU64::new(AiModelState::Unloaded as u64),
            model_loaded: AtomicU64::new(0),
            inference_count: AtomicU64::new(0),
            total_latency_us: AtomicU64::new(0),
        }
    }

    /// Carga el modelo STT desde un archivo o memoria.
    pub unsafe fn load_model(&mut self, model_data: &[u8]) -> AiResult<()> {
        if model_data.is_empty() {
            return Err(AiError::InvalidInput);
        }

        self.state.store(AiModelState::Loading as u64, Ordering::Release);

        // Simular carga del modelo (en producción: inicializar whisper.cpp / Sherpa-ONNX)
        let _ = model_data.len();

        self.model_loaded.store(1, Ordering::Release);
        self.state.store(AiModelState::Ready as u64, Ordering::Release);
        Ok(())
    }

    /// Descarga el modelo STT.
    pub unsafe fn unload_model(&mut self) -> AiResult<()> {
        self.model_loaded.store(0, Ordering::Release);
        self.state.store(AiModelState::Unloaded as u64, Ordering::Release);
        Ok(())
    }

    /// Transcribe audio a texto.
    pub fn transcribe(&mut self, audio_samples: &[i16]) -> AiResult<SttTranscription> {
        if self.model_loaded.load(Ordering::Acquire) == 0 {
            return Err(AiError::ModelNotLoaded);
        }

        if audio_samples.is_empty() {
            return Err(AiError::InvalidInput);
        }

        let start = time_us();

        self.state.store(AiModelState::Inferring as u64, Ordering::Release);

        // Simular inferencia STT (en producción: ejecutar whisper.cpp / Sherpa-ONNX)
        let text = "Hola, este es un texto transcrito por el modelo STT local";
        let confidence = 0.92f32;

        let latency = time_us().saturating_sub(start);

        self.inference_count.fetch_add(1, Ordering::Release);
        self.total_latency_us.fetch_add(latency, Ordering::Release);
        self.state.store(AiModelState::Ready as u64, Ordering::Release);

        let mut text_buf = TextBuffer::new();
        text_buf.set_str(text).unwrap_or(());

        Ok(SttTranscription {
            text: text_buf,
            confidence,
            language_detected: [101, 115, 45, 69, 83, 0, 0, 0], // "es-ES\0\0"
            duration_ms: (audio_samples.len() as u32 * 1000 / self.config.sample_rate),
        })
    }

    /// Devuelve el estado actual del modelo.
    pub fn state(&self) -> AiModelState {
        match self.state.load(Ordering::Acquire) {
            0 => AiModelState::Unloaded,
            1 => AiModelState::Loading,
            2 => AiModelState::Ready,
            3 => AiModelState::Error,
            4 => AiModelState::Inferring,
            _ => AiModelState::Unloaded,
        }
    }

    /// Estadísticas del motor STT.
    pub fn stats(&self) -> (u64, u64) {
        (
            self.inference_count.load(Ordering::Acquire),
            self.total_latency_us.load(Ordering::Acquire),
        )
    }
}

impl Default for SttEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AiModel for SttEngine {
    fn model_type(&self) -> AiModelType {
        AiModelType::STT
    }

    fn state(&self) -> AiModelState {
        self.state()
    }

    fn load(&mut self) -> AiResult<()> {
        // Carga por defecto (modelo embebido placeholder)
        unsafe { self.load_model(b"whisper_tiny_placeholder") }
    }

    fn unload(&mut self) -> AiResult<()> {
        unsafe { self.unload_model() }
    }

    fn infer(&mut self, request: &AiInferenceRequest) -> AiResult<AiInferenceResponse> {
        let start = time_us();

        // Convertir input_data a samples de audio (simplificado: tratar como i16)
        let samples: &[i16] = unsafe {
            core::slice::from_raw_parts(
                request.input_data.as_slice().as_ptr() as *const i16,
                request.input_data.len() / 2,
            )
        };

        let transcription = self.transcribe(samples)?;

        let mut output = AiDataBuffer::new();
        output.push_slice(transcription.text.as_str().as_bytes())?;

        let latency = time_us().saturating_sub(start);

        Ok(AiInferenceResponse {
            request_id: request.request_id,
            model_type: AiModelType::STT,
            output_data: output,
            latency_us: latency,
            success: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stt_config_default() {
        let config = SttConfig::default();
        assert_eq!(config.sample_rate, 16000);
        assert!(config.use_gpu);
    }

    #[test]
    fn stt_engine_new() {
        let engine = SttEngine::new();
        assert_eq!(engine.state(), AiModelState::Unloaded);
    }

    #[test]
    fn stt_load_unload() {
        let mut engine = SttEngine::new();
        assert!(unsafe { engine.load_model(b"model_data") }.is_ok());
        assert_eq!(engine.state(), AiModelState::Ready);
        assert!(unsafe { engine.unload_model() }.is_ok());
        assert_eq!(engine.state(), AiModelState::Unloaded);
    }

    #[test]
    fn stt_transcribe_not_loaded() {
        let mut engine = SttEngine::new();
        match engine.transcribe(&[0i16; 100]) {
            Err(AiError::ModelNotLoaded) => {},
            _ => panic!("expected ModelNotLoaded"),
        }
    }

    #[test]
    fn stt_transcribe_empty() {
        let mut engine = SttEngine::new();
        unsafe { engine.load_model(b"model").unwrap(); }
        match engine.transcribe(&[]) {
            Err(AiError::InvalidInput) => {},
            _ => panic!("expected InvalidInput"),
        }
    }

    #[test]
    fn stt_transcribe_success() {
        let mut engine = SttEngine::new();
        unsafe { engine.load_model(b"model").unwrap(); }
        let result = engine.transcribe(&[1i16, 2, 3, 4, 5]);
        assert!(result.is_ok());
        let t = result.unwrap();
        assert!(!t.text.is_empty());
        assert!(t.confidence > 0.0);
    }

    #[test]
    fn stt_text_buffer() {
        let mut tb = TextBuffer::new();
        assert!(tb.set_str("hola").is_ok());
        assert_eq!(tb.as_str(), "hola");
        assert_eq!(tb.len(), 4);
    }

    #[test]
    fn stt_ai_model_trait() {
        let mut engine = SttEngine::new();
        assert_eq!(engine.model_type(), AiModelType::STT);
        assert!(engine.load().is_ok());
        assert_eq!(engine.state(), AiModelState::Ready);
    }
}
