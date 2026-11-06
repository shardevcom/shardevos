//! TTS (Text-to-Speech) — Síntesis de Texto a Voz.
//!
//! Integración con Piper TTS para generación de audio de voz
//! desde texto, procesado vía GPU/NPU en espacio de usuario.

use super::{AiModel, AiModelType, AiModelState, AiResult, AiError, AiInferenceRequest, AiInferenceResponse, AiDataBuffer, time_us};
use core::sync::atomic::{AtomicU64, Ordering};

/// Configuración del modelo TTS.
#[derive(Debug, Clone, Copy)]
pub struct TtsConfig {
    pub sample_rate: u32,
    pub speaker_id: u32,
    pub noise_scale: f32,
    pub length_scale: f32,
    pub use_gpu: bool,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            sample_rate: 22050,
            speaker_id: 0,
            noise_scale: 0.667,
            length_scale: 1.0,
            use_gpu: true,
        }
    }
}

/// Buffer de audio PCM sintetizado.
#[derive(Debug, Clone)]
pub struct AudioBuffer {
    samples: [i16; 4096],
    len: usize,
    sample_rate: u32,
}

impl AudioBuffer {
    pub const fn new() -> Self {
        Self { samples: [0; 4096], len: 0, sample_rate: 22050 }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn as_slice(&self) -> &[i16] {
        &self.samples[..self.len]
    }

    pub fn as_bytes(&self) -> &[u8] {
        unsafe {
            core::slice::from_raw_parts(
                self.samples.as_ptr() as *const u8,
                self.len * 2,
            )
        }
    }

    pub fn set_samples(&mut self, samples: &[i16], sample_rate: u32) -> Result<(), AiError> {
        if samples.len() > 4096 {
            return Err(AiError::OutOfMemory);
        }
        self.samples[..samples.len()].copy_from_slice(samples);
        self.len = samples.len();
        self.sample_rate = sample_rate;
        Ok(())
    }

    pub fn duration_ms(&self) -> u32 {
        if self.sample_rate == 0 {
            return 0;
        }
        (self.len as u32 * 1000) / self.sample_rate
    }
}

impl Default for AudioBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Motor TTS (Piper TTS).
pub struct TtsEngine {
    config: TtsConfig,
    state: AtomicU64,
    model_loaded: AtomicU64,
    inference_count: AtomicU64,
    total_latency_us: AtomicU64,
    total_audio_ms: AtomicU64,
}

impl TtsEngine {
    pub fn new() -> Self {
        Self {
            config: TtsConfig::default(),
            state: AtomicU64::new(AiModelState::Unloaded as u64),
            model_loaded: AtomicU64::new(0),
            inference_count: AtomicU64::new(0),
            total_latency_us: AtomicU64::new(0),
            total_audio_ms: AtomicU64::new(0),
        }
    }

    /// Carga el modelo TTS (Piper).
    pub unsafe fn load_model(&mut self, model_data: &[u8]) -> AiResult<()> {
        if model_data.is_empty() {
            return Err(AiError::InvalidInput);
        }

        self.state.store(AiModelState::Loading as u64, Ordering::Release);
        let _ = model_data.len();
        self.model_loaded.store(1, Ordering::Release);
        self.state.store(AiModelState::Ready as u64, Ordering::Release);
        Ok(())
    }

    /// Descarga el modelo TTS.
    pub unsafe fn unload_model(&mut self) -> AiResult<()> {
        self.model_loaded.store(0, Ordering::Release);
        self.state.store(AiModelState::Unloaded as u64, Ordering::Release);
        Ok(())
    }

    /// Sintetiza texto a audio.
    pub fn synthesize(&mut self, text: &str) -> AiResult<AudioBuffer> {
        if self.model_loaded.load(Ordering::Acquire) == 0 {
            return Err(AiError::ModelNotLoaded);
        }

        if text.is_empty() {
            return Err(AiError::InvalidInput);
        }

        let start = time_us();

        self.state.store(AiModelState::Inferring as u64, Ordering::Release);

        // Simular síntesis TTS (en producción: ejecutar Piper TTS)
        let audio = Self::mock_synthesize(text, self.config.sample_rate);

        let latency = time_us().saturating_sub(start);
        let duration = audio.duration_ms() as u64;

        self.inference_count.fetch_add(1, Ordering::Release);
        self.total_latency_us.fetch_add(latency, Ordering::Release);
        self.total_audio_ms.fetch_add(duration, Ordering::Release);
        self.state.store(AiModelState::Ready as u64, Ordering::Release);

        Ok(audio)
    }

    /// Síntesis simulada (placeholder para Piper TTS).
    fn mock_synthesize(_text: &str, sample_rate: u32) -> AudioBuffer {
        // Generar un patrón de audio simple como placeholder (sin trigonometría en no_std)
        let mut buf = AudioBuffer::new();
        let sample_count = (sample_rate as usize * 2).min(4096); // 2 segundos de audio
        let mut samples = [0i16; 4096];
        for i in 0..sample_count {
            // Patrón simple de onda cuadrada
            let cycle = (i % (sample_rate as usize / 100)) as i32;
            samples[i] = if cycle < (sample_rate as i32 / 200) { 16384 } else { -16384 };
        }
        buf.set_samples(&samples[..sample_count], sample_rate).unwrap();
        buf
    }

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

    pub fn stats(&self) -> (u64, u64, u64) {
        (
            self.inference_count.load(Ordering::Acquire),
            self.total_latency_us.load(Ordering::Acquire),
            self.total_audio_ms.load(Ordering::Acquire),
        )
    }
}

impl Default for TtsEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AiModel for TtsEngine {
    fn model_type(&self) -> AiModelType {
        AiModelType::TTS
    }

    fn state(&self) -> AiModelState {
        self.state()
    }

    fn load(&mut self) -> AiResult<()> {
        unsafe { self.load_model(b"piper_es_voice_placeholder") }
    }

    fn unload(&mut self) -> AiResult<()> {
        unsafe { self.unload_model() }
    }

    fn infer(&mut self, request: &AiInferenceRequest) -> AiResult<AiInferenceResponse> {
        let start = time_us();

        let text = core::str::from_utf8(request.input_data.as_slice())
            .map_err(|_| AiError::InvalidInput)?;

        let audio = self.synthesize(text)?;

        let mut output = AiDataBuffer::new();
        output.push_slice(audio.as_bytes())?;

        let latency = time_us().saturating_sub(start);

        Ok(AiInferenceResponse {
            request_id: request.request_id,
            model_type: AiModelType::TTS,
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
    fn tts_config_default() {
        let config = TtsConfig::default();
        assert_eq!(config.sample_rate, 22050);
        assert_eq!(config.speaker_id, 0);
        assert!(config.use_gpu);
    }

    #[test]
    fn tts_engine_new() {
        let engine = TtsEngine::new();
        assert_eq!(engine.state(), AiModelState::Unloaded);
    }

    #[test]
    fn tts_load_unload() {
        let mut engine = TtsEngine::new();
        assert!(unsafe { engine.load_model(b"model") }.is_ok());
        assert_eq!(engine.state(), AiModelState::Ready);
        assert!(unsafe { engine.unload_model() }.is_ok());
        assert_eq!(engine.state(), AiModelState::Unloaded);
    }

    #[test]
    fn tts_synthesize_not_loaded() {
        let mut engine = TtsEngine::new();
        match engine.synthesize("hola") {
            Err(AiError::ModelNotLoaded) => {},
            _ => panic!("expected ModelNotLoaded"),
        }
    }

    #[test]
    fn tts_synthesize_empty() {
        let mut engine = TtsEngine::new();
        unsafe { engine.load_model(b"model").unwrap(); }
        match engine.synthesize("") {
            Err(AiError::InvalidInput) => {},
            _ => panic!("expected InvalidInput"),
        }
    }

    #[test]
    fn tts_synthesize_success() {
        let mut engine = TtsEngine::new();
        unsafe { engine.load_model(b"model").unwrap(); }
        let audio = engine.synthesize("hola mundo").unwrap();
        assert!(!audio.is_empty());
        assert!(audio.duration_ms() > 0);
    }

    #[test]
    fn tts_audio_buffer() {
        let mut buf = AudioBuffer::new();
        let samples = [1i16; 4000];
        assert!(buf.set_samples(&samples, 16000).is_ok());
        assert_eq!(buf.len(), 4000);
        assert_eq!(buf.duration_ms(), 250); // 4000/16000 * 1000 = 250
    }

    #[test]
    fn tts_ai_model_trait() {
        let mut engine = TtsEngine::new();
        assert_eq!(engine.model_type(), AiModelType::TTS);
        assert!(engine.load().is_ok());
        assert_eq!(engine.state(), AiModelState::Ready);
    }
}
