//! SLM (Small Language Model) — Modelo de Lenguaje Pequeño con Function Calling.
//!
//! Integración de SLM local (tipo Llama-3-3B / Phi-3) con soporte para
//! Function Calling directo sobre las APIs del sistema operativo.

use super::{AiModel, AiModelType, AiModelState, AiResult, AiError, AiInferenceRequest, AiInferenceResponse, AiDataBuffer, time_us, next_request_id};
use core::sync::atomic::{AtomicU64, Ordering};

/// Configuración del SLM.
#[derive(Debug, Clone, Copy)]
pub struct SlmConfig {
    pub context_length: usize,
    pub temperature: f32,
    pub top_p: f32,
    pub max_tokens: u32,
    pub use_gpu: bool,
    pub enable_function_calling: bool,
}

impl Default for SlmConfig {
    fn default() -> Self {
        Self {
            context_length: 2048,
            temperature: 0.7,
            top_p: 0.9,
            max_tokens: 256,
            use_gpu: true,
            enable_function_calling: true,
        }
    }
}

/// Registro de una función llamable desde el SLM.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FunctionDef {
    pub name: [u8; 64],
    pub description: [u8; 256],
    pub parameters_schema: [u8; 512],
    pub handler: FunctionHandler,
}

/// Tipo de manejador de función del sistema.
pub type FunctionHandler = fn(&[u8]) -> AiResult<HeaplessVec>;

/// Buffer de bytes simple (reemplaza Vec<u8>).
#[derive(Debug, Clone)]
pub struct HeaplessVec {
    data: [u8; 1024],
    len: usize,
}

impl HeaplessVec {
    pub const fn new() -> Self {
        Self { data: [0; 1024], len: 0 }
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

    pub fn from_slice(src: &[u8]) -> Result<Self, AiError> {
        if src.len() > 1024 {
            return Err(AiError::OutOfMemory);
        }
        let mut v = Self::new();
        v.data[..src.len()].copy_from_slice(src);
        v.len = src.len();
        Ok(v)
    }

    pub fn push_slice(&mut self, src: &[u8]) -> Result<(), AiError> {
        if self.len + src.len() > 1024 {
            return Err(AiError::OutOfMemory);
        }
        self.data[self.len..self.len + src.len()].copy_from_slice(src);
        self.len += src.len();
        Ok(())
    }
}

impl Default for HeaplessVec {
    fn default() -> Self {
        Self::new()
    }
}

/// Llamada a función solicitada por el SLM.
#[derive(Debug, Clone)]
#[repr(C)]
pub struct FunctionCall {
    pub function_name: [u8; 64],
    pub arguments: AiDataBuffer,
    pub call_id: u64,
}

/// Resultado de una llamada a función.
#[derive(Debug, Clone)]
#[repr(C)]
pub struct FunctionResult {
    pub call_id: u64,
    pub success: bool,
    pub result_data: AiDataBuffer,
    pub error: Option<AiError>,
}

/// Respuesta de texto del SLM.
#[derive(Debug, Clone)]
pub struct SlmResponse {
    pub text: TextBufferSlm,
    pub function_calls: [FunctionCall; 8],
    pub function_call_count: u8,
    pub finish_reason: FinishReason,
}

/// Motivo de finalización de la generación.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FinishReason {
    Stop = 0,
    Length = 1,
    FunctionCall = 2,
    Error = 3,
}

/// Buffer de texto para SLM.
#[derive(Debug, Clone)]
pub struct TextBufferSlm {
    data: [u8; 1024],
    len: usize,
}

impl TextBufferSlm {
    pub const fn new() -> Self {
        Self { data: [0; 1024], len: 0 }
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
        if s.len() > 1024 {
            return Err(AiError::OutOfMemory);
        }
        self.data[..s.len()].copy_from_slice(s.as_bytes());
        self.len = s.len();
        Ok(())
    }
}

impl Default for TextBufferSlm {
    fn default() -> Self {
        Self::new()
    }
}

/// Registro de funciones disponibles para Function Calling.
struct FunctionRegistry {
    functions: [Option<FunctionDef>; 32],
    count: u8,
}

impl FunctionRegistry {
    const fn new() -> Self {
        Self { functions: [None; 32], count: 0 }
    }

    fn register(&mut self, name: &str, desc: &str, schema: &str, handler: FunctionHandler) -> Result<(), AiError> {
        if self.count >= 32 {
            return Err(AiError::OutOfMemory);
        }
        if name.len() > 64 || desc.len() > 256 || schema.len() > 512 {
            return Err(AiError::InvalidInput);
        }

        let mut n = [0u8; 64];
        n[..name.len()].copy_from_slice(name.as_bytes());
        let mut d = [0u8; 256];
        d[..desc.len()].copy_from_slice(desc.as_bytes());
        let mut s = [0u8; 512];
        s[..schema.len()].copy_from_slice(schema.as_bytes());

        self.functions[self.count as usize] = Some(FunctionDef {
            name: n,
            description: d,
            parameters_schema: s,
            handler,
        });
        self.count += 1;
        Ok(())
    }

    fn find(&self, name: &str) -> Option<&FunctionDef> {
        for f in self.functions.iter().flatten() {
            let fname = core::str::from_utf8(&f.name).unwrap_or("");
            if fname.trim_end_matches(char::from(0)) == name {
                return Some(f);
            }
        }
        None
    }
}

/// SpinLock simple para SLM
mod spin_lock_slm {
    use core::sync::atomic::{AtomicUsize, Ordering};
    use core::cell::UnsafeCell;

    pub struct SpinLock<T> {
        lock: AtomicUsize,
        value: UnsafeCell<T>,
    }

    unsafe impl<T> Send for SpinLock<T> {}
    unsafe impl<T> Sync for SpinLock<T> {}

    impl<T> SpinLock<T> {
        pub const fn new(value: T) -> Self {
            Self { lock: AtomicUsize::new(0), value: UnsafeCell::new(value) }
        }

        pub fn lock(&self) -> SpinLockGuard<'_, T> {
            while self.lock.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).is_err() {
                core::hint::spin_loop();
            }
            SpinLockGuard { lock: self }
        }

        pub fn unlock(&self) {
            self.lock.store(0, Ordering::Release);
        }
    }

    pub struct SpinLockGuard<'a, T> {
        lock: &'a SpinLock<T>,
    }

    impl<'a, T> Drop for SpinLockGuard<'a, T> {
        fn drop(&mut self) {
            self.lock.unlock();
        }
    }

    impl<'a, T> core::ops::Deref for SpinLockGuard<'a, T> {
        type Target = T;
        fn deref(&self) -> &T {
            unsafe { &*self.lock.value.get() }
        }
    }

    impl<'a, T> core::ops::DerefMut for SpinLockGuard<'a, T> {
        fn deref_mut(&mut self) -> &mut T {
            unsafe { &mut *self.lock.value.get() }
        }
    }
}

/// Motor SLM (Llama-3-3B / Phi-3) con Function Calling.
#[allow(dead_code)]
pub struct SlmEngine {
    config: SlmConfig,
    state: AtomicU64,
    model_loaded: AtomicU64,
    inference_count: AtomicU64,
    total_latency_us: AtomicU64,
    function_call_count: AtomicU64,
    registry: spin_lock_slm::SpinLock<FunctionRegistry>,
}

impl SlmEngine {
    pub fn new() -> Self {
        Self {
            config: SlmConfig::default(),
            state: AtomicU64::new(AiModelState::Unloaded as u64),
            model_loaded: AtomicU64::new(0),
            inference_count: AtomicU64::new(0),
            total_latency_us: AtomicU64::new(0),
            function_call_count: AtomicU64::new(0),
            registry: spin_lock_slm::SpinLock::new(FunctionRegistry::new()),
        }
    }

    /// Carga el modelo SLM.
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

    /// Descarga el modelo SLM.
    pub unsafe fn unload_model(&mut self) -> AiResult<()> {
        self.model_loaded.store(0, Ordering::Release);
        self.state.store(AiModelState::Unloaded as u64, Ordering::Release);
        Ok(())
    }

    /// Registra una función del sistema para Function Calling.
    pub fn register_function(&mut self, name: &str, desc: &str, schema: &str, handler: FunctionHandler) -> Result<(), AiError> {
        let mut reg = self.registry.lock();
        reg.register(name, desc, schema, handler)
    }

    /// Ejecuta inferencia del SLM con posible Function Calling.
    pub fn generate(&mut self, prompt: &str) -> AiResult<SlmResponse> {
        if self.model_loaded.load(Ordering::Acquire) == 0 {
            return Err(AiError::ModelNotLoaded);
        }

        if prompt.is_empty() {
            return Err(AiError::InvalidInput);
        }

        let start = time_us();

        self.state.store(AiModelState::Inferring as u64, Ordering::Release);

        // Simular inferencia SLM (en producción: ejecutar llama.cpp / phi-3)
        let (text, calls, call_count) = Self::mock_generate(prompt, &self.registry);

        let latency = time_us().saturating_sub(start);

        self.inference_count.fetch_add(1, Ordering::Release);
        self.total_latency_us.fetch_add(latency, Ordering::Release);
        self.function_call_count.fetch_add(call_count as u64, Ordering::Release);
        self.state.store(AiModelState::Ready as u64, Ordering::Release);

        let mut text_buf = TextBufferSlm::new();
        text_buf.set_str(text).unwrap_or(());

        let mut fc_array = core::array::from_fn(|_| FunctionCall::empty());
        for i in 0..call_count.min(8) {
            fc_array[i] = calls[i].clone();
        }

        let finish_reason = if call_count == 0 {
            FinishReason::Stop
        } else {
            FinishReason::FunctionCall
        };

        Ok(SlmResponse {
            text: text_buf,
            function_calls: fc_array,
            function_call_count: call_count as u8,
            finish_reason,
        })
    }

    /// Generación simulada con detección de intents de función.
    fn mock_generate(prompt: &str, registry: &spin_lock_slm::SpinLock<FunctionRegistry>) -> (&'static str, [FunctionCall; 8], usize) {
        let _lower = prompt; // En producción se haría lowercase real
        let mut calls = core::array::from_fn(|_| FunctionCall::empty());
        let mut call_count = 0usize;

        // Detectar keywords de funciones del sistema
        if prompt.contains("leer archivo") || prompt.contains("read file") {
            if let Some(_f) = registry.lock().find("sys_read") {
                if call_count < 8 {
                    let mut args = AiDataBuffer::new();
                    let _ = args.push_slice(b"{\"path\":\"/etc/hostname\"}");
                    calls[call_count] = FunctionCall {
                        function_name: Self::name_to_array("sys_read"),
                        arguments: args,
                        call_id: next_request_id(),
                    };
                    call_count += 1;
                }
            }
        }

        if prompt.contains("escribir") || prompt.contains("write") {
            if let Some(_f) = registry.lock().find("sys_write") {
                if call_count < 8 {
                    let mut args = AiDataBuffer::new();
                    let _ = args.push_slice(b"{\"fd\":1,\"data\":\"hola desde SLM\"}");
                    calls[call_count] = FunctionCall {
                        function_name: Self::name_to_array("sys_write"),
                        arguments: args,
                        call_id: next_request_id(),
                    };
                    call_count += 1;
                }
            }
        }

        let response_text = if call_count == 0 {
            "He recibido tu mensaje. ¿En qué puedo ayudarte?"
        } else {
            "Voy a ejecutar función(es) del sistema para ti."
        };

        (response_text, calls, call_count)
    }

    /// Convierte un nombre de función a array de 64 bytes.
    fn name_to_array(name: &str) -> [u8; 64] {
        let mut arr = [0u8; 64];
        let bytes = name.as_bytes();
        let copy_len = bytes.len().min(64);
        arr[..copy_len].copy_from_slice(&bytes[..copy_len]);
        arr
    }

    /// Ejecuta una llamada a función devuelta por el SLM.
    pub fn execute_function_call(&self, call: &FunctionCall) -> AiResult<FunctionResult> {
        let start_call_id = call.call_id;
        let reg = self.registry.lock();

        let name_str = core::str::from_utf8(&call.function_name)
            .unwrap_or("")
            .trim_end_matches(char::from(0));

        match reg.find(name_str) {
            Some(func_def) => {
                let result = (func_def.handler)(call.arguments.as_slice());
                match result {
                    Ok(data) => {
                        let mut result_data = AiDataBuffer::new();
                        let _ = result_data.push_slice(data.as_slice());
                        self.function_call_count.fetch_add(1, Ordering::Release);
                        Ok(FunctionResult {
                            call_id: start_call_id,
                            success: true,
                            result_data,
                            error: None,
                        })
                    }
                    Err(e) => Ok(FunctionResult {
                        call_id: start_call_id,
                        success: false,
                        result_data: AiDataBuffer::new(),
                        error: Some(e),
                    }),
                }
            }
            None => Ok(FunctionResult {
                call_id: start_call_id,
                success: false,
                result_data: AiDataBuffer::new(),
                error: Some(AiError::FunctionCallFailed),
            }),
        }
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
            self.function_call_count.load(Ordering::Acquire),
        )
    }
}

impl FunctionCall {
    pub const fn empty() -> Self {
        Self {
            function_name: [0u8; 64],
            arguments: AiDataBuffer::new(),
            call_id: 0,
        }
    }
}

impl Default for SlmEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AiModel for SlmEngine {
    fn model_type(&self) -> AiModelType {
        AiModelType::SLM
    }

    fn state(&self) -> AiModelState {
        self.state()
    }

    fn load(&mut self) -> AiResult<()> {
        unsafe { self.load_model(b"phi-3-mini_placeholder") }
    }

    fn unload(&mut self) -> AiResult<()> {
        unsafe { self.unload_model() }
    }

    fn infer(&mut self, request: &AiInferenceRequest) -> AiResult<AiInferenceResponse> {
        let start = time_us();

        let prompt = core::str::from_utf8(request.input_data.as_slice())
            .map_err(|_| AiError::InvalidInput)?;

        let response = self.generate(prompt)?;

        let mut output = AiDataBuffer::new();
        output.push_slice(response.text.as_str().as_bytes())?;

        let latency = time_us().saturating_sub(start);

        Ok(AiInferenceResponse {
            request_id: request.request_id,
            model_type: AiModelType::SLM,
            output_data: output,
            latency_us: latency,
            success: true,
        })
    }
}

/// Funciones de sistema integradas para Function Calling.

/// Lee un archivo del sistema (placeholder).
fn sys_fn_read(_args: &[u8]) -> AiResult<HeaplessVec> {
    HeaplessVec::from_slice(b"contenido_del_archivo_placeholder")
}

/// Escribe al sistema (placeholder).
fn sys_fn_write(args: &[u8]) -> AiResult<HeaplessVec> {
    let _ = args.len();
    HeaplessVec::from_slice(b"escrito_ok")
}

/// Obtiene información del sistema.
fn sys_fn_uname(_args: &[u8]) -> AiResult<HeaplessVec> {
    HeaplessVec::from_slice(b"shardevos 0.1.0 x86_64")
}

/// Instancia global del SLM.
static SLM_ENGINE: spin_lock_global::SpinLock<Option<SlmEngine>> = spin_lock_global::SpinLock::new(None);

// SpinLock global simple
mod spin_lock_global {
    use core::sync::atomic::{AtomicUsize, Ordering};
    use core::cell::UnsafeCell;

    pub struct SpinLock<T> {
        lock: AtomicUsize,
        value: UnsafeCell<T>,
    }

    unsafe impl<T> Send for SpinLock<T> {}
    unsafe impl<T> Sync for SpinLock<T> {}

    impl<T> SpinLock<T> {
        pub const fn new(value: T) -> Self {
            Self { lock: AtomicUsize::new(0), value: UnsafeCell::new(value) }
        }

        pub fn lock(&self) -> SpinLockGuard<'_, T> {
            while self.lock.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).is_err() {
                core::hint::spin_loop();
            }
            SpinLockGuard { lock: self }
        }

        pub fn unlock(&self) {
            self.lock.store(0, Ordering::Release);
        }
    }

    pub struct SpinLockGuard<'a, T> {
        lock: &'a SpinLock<T>,
    }

    impl<'a, T> Drop for SpinLockGuard<'a, T> {
        fn drop(&mut self) {
            self.lock.unlock();
        }
    }

    impl<'a, T> core::ops::Deref for SpinLockGuard<'a, T> {
        type Target = T;
        fn deref(&self) -> &T {
            unsafe { &*self.lock.value.get() }
        }
    }

    impl<'a, T> core::ops::DerefMut for SpinLockGuard<'a, T> {
        fn deref_mut(&mut self) -> &mut T {
            unsafe { &mut *self.lock.value.get() }
        }
    }
}

/// Obtiene una referencia mutable al SLM global.
pub fn global_slm() -> spin_lock_global::SpinLockGuard<'static, Option<SlmEngine>> {
    SLM_ENGINE.lock()
}

/// Inicializa el SLM global con funciones de sistema registradas.
pub unsafe fn init_global_slm() -> AiResult<()> {
    let mut slm = SLM_ENGINE.lock();
    if slm.is_some() {
        return Err(AiError::NotInitialized);
    }

    let mut engine = SlmEngine::new();
    engine.load()?;

    // Registrar funciones de sistema
    engine.register_function(
        "sys_read",
        "Lee un archivo del sistema",
        "{\"type\":\"object\",\"properties\":{\"path\":{\"type\":\"string\"}}}",
        sys_fn_read,
    )?;
    engine.register_function(
        "sys_write",
        "Escribe datos al sistema",
        "{\"type\":\"object\",\"properties\":{\"fd\":{\"type\":\"integer\"},\"data\":{\"type\":\"string\"}}}",
        sys_fn_write,
    )?;
    engine.register_function(
        "sys_uname",
        "Obtiene información del sistema",
        "{\"type\":\"object\",\"properties\":{}}",
        sys_fn_uname,
    )?;

    *slm = Some(engine);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slm_config_default() {
        let config = SlmConfig::default();
        assert_eq!(config.context_length, 2048);
        assert!((config.temperature - 0.7).abs() < 0.001);
        assert!(config.enable_function_calling);
    }

    #[test]
    fn slm_engine_new() {
        let engine = SlmEngine::new();
        assert_eq!(engine.state(), AiModelState::Unloaded);
    }

    #[test]
    fn slm_load_unload() {
        let mut engine = SlmEngine::new();
        assert!(unsafe { engine.load_model(b"model") }.is_ok());
        assert_eq!(engine.state(), AiModelState::Ready);
        assert!(unsafe { engine.unload_model() }.is_ok());
        assert_eq!(engine.state(), AiModelState::Unloaded);
    }

    #[test]
    fn slm_register_function() {
        let mut engine = SlmEngine::new();
        assert!(engine.register_function(
            "test_fn",
            "una función de prueba",
            "{}",
            |_| Ok(HeaplessVec::from_slice(b"ok").unwrap()),
        ).is_ok());
    }

    #[test]
    fn slm_generate_not_loaded() {
        let mut engine = SlmEngine::new();
        match engine.generate("hola") {
            Err(AiError::ModelNotLoaded) => {},
            _ => panic!("expected ModelNotLoaded"),
        }
    }

    #[test]
    fn slm_generate_success() {
        let mut engine = SlmEngine::new();
        unsafe { engine.load_model(b"model").unwrap(); }
        let resp = engine.generate("hola mundo").unwrap();
        assert!(!resp.text.is_empty());
        assert_eq!(resp.finish_reason, FinishReason::Stop);
    }

    #[test]
    fn slm_function_call_detection() {
        let mut engine = SlmEngine::new();
        unsafe { engine.load_model(b"model").unwrap(); }
        engine.register_function(
            "sys_read",
            "leer archivo",
            "{}",
            |_| Ok(HeaplessVec::from_slice(b"contenido").unwrap()),
        ).unwrap();

        let resp = engine.generate("leer archivo de configuración").unwrap();
        assert!(resp.function_call_count > 0);
        assert_eq!(resp.finish_reason, FinishReason::FunctionCall);
    }

    #[test]
    fn slm_execute_function_call() {
        let mut engine = SlmEngine::new();
        unsafe { engine.load_model(b"model").unwrap(); }
        engine.register_function(
            "sys_read",
            "leer archivo",
            "{}",
            |_| Ok(HeaplessVec::from_slice(b"resultado_lectura").unwrap()),
        ).unwrap();

        let mut args = AiDataBuffer::new();
        let _ = args.push_slice(b"{}");
        let call = FunctionCall {
            function_name: SlmEngine::name_to_array("sys_read"),
            arguments: args,
            call_id: 42,
        };

        let result = engine.execute_function_call(&call).unwrap();
        assert!(result.success);
        assert_eq!(result.call_id, 42);
        assert!(!result.result_data.is_empty());
    }

    #[test]
    fn slm_execute_unknown_function() {
        let engine = SlmEngine::new();
        let mut args = AiDataBuffer::new();
        let _ = args.push_slice(b"{}");
        let call = FunctionCall {
            function_name: SlmEngine::name_to_array("func_inexistente"),
            arguments: args,
            call_id: 99,
        };

        let result = engine.execute_function_call(&call).unwrap();
        assert!(!result.success);
        assert_eq!(result.error, Some(AiError::FunctionCallFailed));
    }

    #[test]
    fn slm_text_buffer() {
        let mut buf = TextBufferSlm::new();
        assert!(buf.set_str("hola slm").is_ok());
        assert_eq!(buf.as_str(), "hola slm");
        assert_eq!(buf.len(), 8);
    }

    #[test]
    fn slm_ai_model_trait() {
        let mut engine = SlmEngine::new();
        assert_eq!(engine.model_type(), AiModelType::SLM);
        assert!(engine.load().is_ok());
        assert_eq!(engine.state(), AiModelState::Ready);
    }

    #[test]
    fn heapless_vec_ops() {
        let mut v = HeaplessVec::new();
        assert!(v.push_slice(b"hello").is_ok());
        assert_eq!(v.len(), 5);
        assert_eq!(v.as_slice(), b"hello");
    }
}
