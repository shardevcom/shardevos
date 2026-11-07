//! Embeddings Quantizados (INT8) — Motor de embeddings para espacio de usuario.
//!
//! Versión standalone para usuariospace (no_std). Convierte texto en vectores
//! numéricos para búsqueda semántica. Usa cuantización INT8.
//!
//! Nota: Este módulo es una copia simplificada del módulo del kernel
//! (kernel/ai_engine/embeddings.rs) para funcionar en espacio de usuario
//! sin depender de crates del kernel.

/// Dimensión del vector de embedding.
pub const EMBEDDING_DIM: usize = 64;

/// Número máximo de tokens por texto.
const MAX_TOKENS: usize = 256;

/// Tamaño del vocabulario para el modelo de embeddings simulado.
const VOCAB_SIZE: usize = 512;

/// Vector de embedding (INT8 quantizado).
#[derive(Debug, Clone, Copy)]
pub struct Embedding {
    /// Valores quantizados int8 (-128 a 127).
    values: [i8; EMBEDDING_DIM],
    /// Factor de escala para desquantizar.
    scale: f32,
    /// Punto cero para desquantizar.
    zero_point: i8,
}

impl Embedding {
    /// Crea un embedding vacío (todos ceros).
    pub const fn zero() -> Self {
        Self {
            values: [0; EMBEDDING_DIM],
            scale: 1.0,
            zero_point: 0,
        }
    }

    /// Crea un embedding desde valores f32 normalizados.
    pub fn from_f32(values: &[f32]) -> Self {
        let mut emb = Self::zero();
        if values.len() != EMBEDDING_DIM {
            return emb;
        }

        // Calcular min/max para cuantización.
        let mut min_val = f32::INFINITY;
        let mut max_val = f32::NEG_INFINITY;
        for &v in values.iter() {
            if v < min_val {
                min_val = v;
            }
            if v > max_val {
                max_val = v;
            }
        }

        let range = max_val - min_val;
        let scale = if range > 0.0 { range / 255.0 } else { 1.0 };
        let zero_point = round_f32(-min_val / scale).clamp(-128.0, 127.0) as i8;

        for i in 0..EMBEDDING_DIM {
            let quantized = round_f32(values[i] / scale) as i32 + zero_point as i32;
            emb.values[i] = quantized.clamp(-128, 127) as i8;
        }
        emb.scale = scale;
        emb.zero_point = zero_point;

        emb
    }

    /// Convierte el valor quantizado de vuelta a f32.
    fn dequantize(&self, idx: usize) -> f32 {
        (self.values[idx] as f32 - self.zero_point as f32) * self.scale
    }

    /// Obtiene el valor en la posición dada (desquantizado).
    pub fn get(&self, idx: usize) -> f32 {
        if idx < EMBEDDING_DIM {
            self.dequantize(idx)
        } else {
            0.0
        }
    }

    /// Calcula la similitud de coseno con otro embedding.
    pub fn cosine_similarity(&self, other: &Embedding) -> f32 {
        let mut dot_product: f32 = 0.0;
        let mut norm_a: f32 = 0.0;
        let mut norm_b: f32 = 0.0;

        for i in 0..EMBEDDING_DIM {
            let a = self.dequantize(i);
            let b = other.dequantize(i);
            dot_product += a * b;
            norm_a += a * a;
            norm_b += b * b;
        }

        let norm_a = sqrt_f32(norm_a);
        let norm_b = sqrt_f32(norm_b);

        if norm_a == 0.0 || norm_b == 0.0 {
            return 0.0;
        }

        dot_product / (norm_a * norm_b)
    }

    /// Verifica si el embedding es cero (no inicializado).
    pub fn is_zero(&self) -> bool {
        self.values.iter().all(|&v| v == 0)
    }
}

impl Default for Embedding {
    fn default() -> Self {
        Self::zero()
    }
}

/// Tokenizador simple: divide texto en tokens (palabras) y mapea a IDs.
const MAX_WORD_LEN: usize = 32;

/// Palabra del vocabulario almacenada como bytes planos (permite tokens cargados desde el modelo).
#[derive(Debug, Clone, Copy)]
pub struct VocabWord {
    len: u8,
    bytes: [u8; MAX_WORD_LEN],
}

impl VocabWord {
    /// Crea una palabra vacía.
    pub const fn empty() -> Self {
        Self {
            len: 0,
            bytes: [0; MAX_WORD_LEN],
        }
    }

    /// Crea una palabra desde un slice de bytes.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_WORD_LEN {
            return None;
        }
        let mut word = Self::empty();
        word.len = bytes.len() as u8;
        word.bytes[..bytes.len()].copy_from_slice(bytes);
        Some(word)
    }

    /// Devuelve los bytes de la palabra.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

pub struct Tokenizer {
    vocab: [VocabWord; VOCAB_SIZE],
    vocab_size: usize,
}

impl Tokenizer {
    /// Crea un tokenizer vacío.
    pub const fn new() -> Self {
        Self {
            vocab: [VocabWord::empty(); VOCAB_SIZE],
            vocab_size: 0,
        }
    }

    /// Vacía el vocabulario.
    pub fn clear(&mut self) {
        for i in 0..VOCAB_SIZE {
            self.vocab[i] = VocabWord::empty();
        }
        self.vocab_size = 0;
    }

    /// Número de palabras cargadas.
    pub fn len(&self) -> usize {
        self.vocab_size
    }

    /// Agrega una palabra al vocabulario y retorna su ID.
    /// Retorna `VOCAB_SIZE` cuando el vocabulario está lleno o la palabra es demasiado larga.
    pub fn add_word(&mut self, word: &str) -> usize {
        let bytes = word.as_bytes();
        for i in 0..self.vocab_size {
            if self.vocab[i].as_bytes() == bytes {
                return i;
            }
        }
        if self.vocab_size < VOCAB_SIZE {
            if let Some(w) = VocabWord::from_bytes(bytes) {
                let id = self.vocab_size;
                self.vocab[id] = w;
                self.vocab_size += 1;
                return id;
            }
        }
        VOCAB_SIZE
    }

    /// Tokeniza un texto y devuelve los IDs de los tokens.
    pub fn tokenize(&self, text: &str) -> [usize; MAX_TOKENS] {
        // VOCAB_SIZE es el centinela de padding / palabra desconocida.
        let mut tokens = [VOCAB_SIZE; MAX_TOKENS];
        let mut count = 0;

        for word in text.split_whitespace() {
            if count >= MAX_TOKENS {
                break;
            }
            let bytes = word.as_bytes();
            for i in 0..self.vocab_size {
                if self.vocab[i].as_bytes() == bytes {
                    tokens[count] = i;
                    break;
                }
            }
            count += 1;
        }

        tokens
    }
}

/// Errores del formato serializado de embeddings (.semb).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelError {
    InvalidMagic = 0,
    UnsupportedVersion = 1,
    InvalidDimension = 2,
    InvalidVocabSize = 3,
    TruncatedData = 4,
    InvalidUtf8 = 5,
    VocabularyTooLarge = 6,
}

const SEMB_MAGIC: u32 = 0x424D_4553;
const SEMB_VERSION: u16 = 1;
const SEMB_HEADER_LEN: usize = 12;

/// Lee un entero de 16 bits little endian.
fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    if data.len() < offset + 2 {
        return None;
    }
    Some(u16::from_le_bytes([data[offset], data[offset + 1]]))
}

/// Lee un entero de 32 bits little endian.
fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    if data.len() < offset + 4 {
        return None;
    }
    Some(u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]))
}

/// Escribe un entero de 16 bits little endian.
fn write_u16(out: &mut [u8], offset: usize, value: u16) {
    out[offset] = (value & 0xff) as u8;
    out[offset + 1] = ((value >> 8) & 0xff) as u8;
}

/// Escribe un entero de 32 bits little endian.
fn write_u32(out: &mut [u8], offset: usize, value: u32) {
    out[offset] = (value & 0xff) as u8;
    out[offset + 1] = ((value >> 8) & 0xff) as u8;
    out[offset + 2] = ((value >> 16) & 0xff) as u8;
    out[offset + 3] = ((value >> 24) & 0xff) as u8;
}

/// Devuelve los bytes de una palabra, si existe.
fn semb_word_bytes(word: &VocabWord) -> Option<&[u8]> {
    if word.len == 0 {
        return None;
    }
    Some(&word.bytes[..word.len as usize])
}

/// Motor de embeddings: combina tokenizador y modelo de vectores.
pub struct EmbeddingModel {
    tokenizer: Tokenizer,
    /// Matriz de embeddings: vocab_size x EMBEDDING_DIM.
    embedding_matrix: [[i8; EMBEDDING_DIM]; VOCAB_SIZE],
    /// Índice de inicialización.
    initialized: bool,
}

impl EmbeddingModel {
    /// Crea un nuevo modelo de embeddings vacío.
    pub const fn new() -> Self {
        Self {
            tokenizer: Tokenizer::new(),
            embedding_matrix: [[0; EMBEDDING_DIM]; VOCAB_SIZE],
            initialized: false,
        }
    }

    /// Inicializa el modelo con palabras base y sus embeddings.
    pub fn init(&mut self) {
        // Palabras base con embeddings pre-definidos para demostración.
        // En producción, estos vendrían de un modelo entrenado como all-MiniLM-L6-v2.
        let base_words = [
            ("archivo", 0.8, 0.2, -0.1, 0.5),
            ("documento", 0.7, 0.3, 0.0, 0.4),
            ("búsqueda", 0.9, -0.1, 0.2, 0.6),
            ("archivos", 0.75, 0.25, -0.05, 0.45),
            ("sistema", 0.6, 0.1, 0.3, 0.2),
            ("operativo", 0.65, 0.05, 0.35, 0.25),
            ("kernel", 0.7, -0.05, 0.4, 0.3),
            ("rust", 0.5, 0.4, -0.2, 0.1),
            ("assembly", 0.55, 0.35, -0.15, 0.15),
            ("x86", 0.58, 0.3, -0.1, 0.2),
            ("memoria", 0.65, 0.15, 0.2, 0.35),
            ("pagina", 0.62, 0.1, 0.25, 0.3),
            ("paginacion", 0.63, 0.12, 0.22, 0.28),
            ("disco", 0.55, 0.05, 0.1, 0.4),
            ("bloque", 0.52, 0.0, 0.05, 0.35),
            ("red", 0.45, -0.1, -0.05, 0.7),
            ("redes", 0.48, -0.05, 0.0, 0.65),
            ("tcp", 0.42, -0.15, -0.1, 0.68),
            ("ip", 0.4, -0.2, -0.15, 0.65),
            ("arp", 0.38, -0.25, -0.2, 0.62),
            ("icmp", 0.35, -0.3, -0.25, 0.6),
            ("ping", 0.33, -0.35, -0.3, 0.58),
            ("usuario", 0.3, 0.4, 0.5, 0.1),
            ("grupo", 0.28, 0.35, 0.45, 0.15),
            ("permiso", 0.32, 0.38, 0.48, 0.12),
            ("seguridad", 0.35, 0.42, 0.52, 0.1),
            ("autenticacion", 0.38, 0.45, 0.55, 0.05),
            ("login", 0.36, 0.43, 0.53, 0.03),
            ("vga", 0.2, 0.5, 0.6, -0.1),
            ("pantalla", 0.22, 0.45, 0.55, -0.05),
            ("texto", 0.25, 0.42, 0.5, 0.0),
            ("fuente", 0.23, 0.4, 0.45, 0.05),
            ("ai", 0.4, 0.6, 0.3, 0.2),
            ("ia", 0.38, 0.58, 0.28, 0.22),
            ("modelo", 0.42, 0.55, 0.25, 0.15),
            ("embed", 0.45, 0.52, 0.2, 0.1),
            ("embedding", 0.46, 0.53, 0.21, 0.09),
            ("vector", 0.44, 0.5, 0.18, 0.12),
            ("coseno", 0.43, 0.48, 0.15, 0.1),
            ("similitud", 0.41, 0.46, 0.12, 0.08),
            ("motor", 0.37, 0.55, 0.27, 0.19),
            ("motor-ia", 0.38, 0.56, 0.28, 0.2),
        ];

        for (word, v0, v1, v2, v3) in base_words.iter() {
            let word_id = self.tokenizer.add_word(word);
            if word_id < VOCAB_SIZE {
                let mut row = [0i8; EMBEDDING_DIM];
                row[0] = (v0 * 127.0) as i8;
                row[1] = (v1 * 127.0) as i8;
                row[2] = (v2 * 127.0) as i8;
                row[3] = (v3 * 127.0) as i8;
                self.embedding_matrix[word_id] = row;
            }
        }

        self.initialized = true;
    }

    /// Reinicia el modelo para cargar un archivo serializado.
    fn begin_load(&mut self) {
        self.tokenizer.clear();
        for i in 0..VOCAB_SIZE {
            self.embedding_matrix[i] = [0; EMBEDDING_DIM];
        }
        self.initialized = false;
    }

    /// Finaliza una carga serializada.
    fn finish_load(&mut self) {
        self.initialized = true;
    }

    /// Exporta el modelo actual al formato serializado (.semb).
    pub fn export_semb(&self, out: &mut [u8]) -> Option<usize> {
        if !self.initialized {
            return None;
        }

        let mut needed = SEMB_HEADER_LEN;
        for i in 0..self.tokenizer.len() {
            if let Some(w) = semb_word_bytes(&self.tokenizer.vocab[i]) {
                needed += 1 + w.len() + EMBEDDING_DIM;
            }
        }

        if out.len() < needed {
            return None;
        }

        let mut offset = 0;
        write_u32(out, offset, SEMB_MAGIC);
        offset += 4;
        write_u16(out, offset, SEMB_VERSION);
        offset += 2;
        write_u16(out, offset, EMBEDDING_DIM as u16);
        offset += 2;
        write_u32(out, offset, self.tokenizer.len() as u32);
        offset += 4;

        for i in 0..self.tokenizer.len() {
            if let Some(w) = semb_word_bytes(&self.tokenizer.vocab[i]) {
                if offset + 1 + w.len() + EMBEDDING_DIM > out.len() {
                    return None;
                }
                out[offset] = w.len() as u8;
                offset += 1;
                out[offset..offset + w.len()].copy_from_slice(w);
                offset += w.len();
                for j in 0..EMBEDDING_DIM {
                    out[offset + j] = self.embedding_matrix[i][j] as u8;
                    offset += 1;
                }
            }
        }

        Some(offset)
    }

    /// Carga un modelo serializado (.semb) y reemplaza el estado actual.
    pub fn load_from_semb(&mut self, data: &[u8]) -> Result<(), ModelError> {
        if data.len() < SEMB_HEADER_LEN {
            return Err(ModelError::TruncatedData);
        }
        let magic = read_u32(data, 0).ok_or(ModelError::InvalidMagic)?;
        if magic != SEMB_MAGIC {
            return Err(ModelError::InvalidMagic);
        }
        let version = read_u16(data, 4).ok_or(ModelError::InvalidMagic)?;
        if version != SEMB_VERSION {
            return Err(ModelError::UnsupportedVersion);
        }
        let dim = read_u16(data, 6).ok_or(ModelError::InvalidMagic)? as usize;
        if dim != EMBEDDING_DIM {
            return Err(ModelError::InvalidDimension);
        }
        let vocab_size = read_u32(data, 8).ok_or(ModelError::InvalidMagic)? as usize;
        if vocab_size > VOCAB_SIZE {
            return Err(ModelError::InvalidVocabSize);
        }

        self.begin_load();

        let mut offset = SEMB_HEADER_LEN;
        for _ in 0..vocab_size {
            if data.len() < offset + 1 {
                return Err(ModelError::TruncatedData);
            }
            let word_len = data[offset] as usize;
            offset += 1;
            if offset + word_len > data.len() || offset + word_len + EMBEDDING_DIM > data.len() {
                return Err(ModelError::TruncatedData);
            }
            let word = core::str::from_utf8(&data[offset..offset + word_len])
                .map_err(|_| ModelError::InvalidUtf8)?;
            let id = self.tokenizer.add_word(word);
            let row_offset = offset + word_len;
            let mut vector = [0i8; EMBEDDING_DIM];
            for j in 0..EMBEDDING_DIM {
                vector[j] = data[row_offset + j] as i8;
            }
            if id < VOCAB_SIZE {
                self.embedding_matrix[id] = vector;
            }
            offset = row_offset + EMBEDDING_DIM;
        }

        self.finish_load();
        Ok(())
    }

    /// Genera un embedding para un texto dado.
    pub fn embed(&self, text: &str) -> Embedding {
        if !self.initialized {
            return Embedding::zero();
        }

        let tokens = self.tokenizer.tokenize(text);
        let mut sum: [f32; EMBEDDING_DIM] = [0.0; EMBEDDING_DIM];
        let mut count: usize = 0;

        for &token_id in tokens.iter() {
            if token_id >= VOCAB_SIZE {
                continue; // Token padding o palabra desconocida.
            }
            for i in 0..EMBEDDING_DIM {
                sum[i] += self.embedding_matrix[token_id][i] as f32;
            }
            count += 1;
        }

        if count == 0 {
            return Embedding::zero();
        }

        // Promediar los vectores de tokens.
        for i in 0..EMBEDDING_DIM {
            sum[i] /= count as f32;
        }

        Embedding::from_f32(&sum)
    }

    /// Calcula la similitud de coseno entre dos textos.
    pub fn text_similarity(&self, text_a: &str, text_b: &str) -> f32 {
        let emb_a = self.embed(text_a);
        let emb_b = self.embed(text_b);
        emb_a.cosine_similarity(&emb_b)
    }
}

impl Default for EmbeddingModel {
    fn default() -> Self {
        let mut model = Self::new();
        model.init();
        model
    }
}


/// Número máximo de documentos en el índice semántico.
pub const MAX_INDEX_ENTRIES: usize = 256;

/// Resultado de una búsqueda semántica (inodo + similitud).
#[derive(Debug, Clone, Copy)]
pub struct SearchHit {
    pub inode_id: u64,
    pub similarity: f32,
}

impl SearchHit {
    pub const fn empty() -> Self {
        Self {
            inode_id: 0,
            similarity: 0.0,
        }
    }
}

/// Conector de embeddings para búsqueda semántica.
///
/// Mantiene un índice acotado de (inodo, embedding) y resuelve consultas
/// top-k por similitud de coseno sin necesitar allocador en no_std.
pub struct EmbeddingIndex {
    entries: [Option<(u64, Embedding)>; MAX_INDEX_ENTRIES],
    len: usize,
}

impl EmbeddingIndex {
    /// Crea un índice vacío.
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_INDEX_ENTRIES],
            len: 0,
        }
    }

    /// Vacía el índice.
    pub fn clear(&mut self) {
        self.entries = [None; MAX_INDEX_ENTRIES];
        self.len = 0;
    }

    /// Número de documentos indexados.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Devuelve true si el índice está vacío.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Agrega un documento al índice.
    pub fn insert(&mut self, inode_id: u64, embedding: Embedding) -> bool {
        if self.len >= MAX_INDEX_ENTRIES {
            return false;
        }
        self.entries[self.len] = Some((inode_id, embedding));
        self.len += 1;
        true
    }

    /// Busca los `out.len()` documentos más similares a `query`.
    /// Devuelve cuántos resultados se escribieron en `out`.
    pub fn search(&self, query: &Embedding, out: &mut [SearchHit]) -> usize {
        let mut count = 0usize;
        for i in 0..self.len {
            if let Some((inode_id, embedding)) = self.entries[i] {
                let hit = SearchHit {
                    inode_id,
                    similarity: query.cosine_similarity(&embedding),
                };
                insert_hit(out, &mut count, hit);
            }
        }
        count
    }

    /// Genera el embedding de la consulta y busca los documentos más similares.
    pub fn search_text(
        &self,
        model: &EmbeddingModel,
        text: &str,
        out: &mut [SearchHit],
    ) -> usize {
        let query = model.embed(text);
        self.search(&query, out)
    }
}

impl Default for EmbeddingIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// Inserta un resultado manteniendo `out[..count]` ordenado por similitud.
fn insert_hit(out: &mut [SearchHit], count: &mut usize, hit: SearchHit) {
    if out.is_empty() {
        return;
    }
    let limit = out.len();
    if *count < limit {
        let mut pos = *count;
        while pos > 0 && out[pos - 1].similarity < hit.similarity {
            out[pos] = out[pos - 1];
            pos -= 1;
        }
        out[pos] = hit;
        *count += 1;
    } else if hit.similarity > out[limit - 1].similarity {
        let mut pos = limit - 1;
        while pos > 0 && out[pos - 1].similarity < hit.similarity {
            out[pos] = out[pos - 1];
            pos -= 1;
        }
        out[pos] = hit;
    }
}

/// Instancia global del modelo de embeddings para userspace.
static mut EMBEDDING_MODEL: EmbeddingModel = EmbeddingModel::new();

/// Devuelve una referencia mutable al modelo de embeddings global.
pub fn global_embedding_model() -> &'static mut EmbeddingModel {
    unsafe { &mut EMBEDDING_MODEL }
}

/// Inicializa el modelo de embeddings global.
pub unsafe fn init_embedding_model() {
    EMBEDDING_MODEL.init();
}

/// Carga un modelo serializado (.semb) en el modelo global.
pub unsafe fn load_embedding_model_from_bytes(data: &[u8]) -> Result<(), ModelError> {
    EMBEDDING_MODEL.load_from_semb(data)
}

/// Instancia global del índice semántico.
static mut EMBEDDING_INDEX: EmbeddingIndex = EmbeddingIndex::new();

/// Devuelve una referencia mutable al índice semántico global.
pub fn global_embedding_index() -> &'static mut EmbeddingIndex {
    unsafe { &mut EMBEDDING_INDEX }
}

/// Indexa un documento usando el modelo global. Devuelve false si el índice está lleno.
pub fn index_document(inode_id: u64, text: &str) -> bool {
    let embedding = embed_text(text);
    global_embedding_index().insert(inode_id, embedding)
}

/// Resuelve una consulta semántica contra el índice global.
pub fn semantic_search(text: &str, out: &mut [SearchHit]) -> usize {
    unsafe { EMBEDDING_INDEX.search_text(&EMBEDDING_MODEL, text, out) }
}

/// Genera un embedding para un texto.
pub fn embed_text(text: &str) -> Embedding {
    unsafe { EMBEDDING_MODEL.embed(text) }
}

/// Calcula la similitud de coseno entre dos textos.
pub fn text_similarity(text_a: &str, text_b: &str) -> f32 {
    unsafe { EMBEDDING_MODEL.text_similarity(text_a, text_b) }
}

/// Raíz cuadrada aproximada (Newton-Raphson) para no_std sin f32::sqrt.
fn sqrt_f32(x: f32) -> f32 {
    if x == 0.0 {
        return 0.0;
    }
    if x < 0.0 {
        return 0.0;
    }
    let mut g = x / 2.0;
    if g == 0.0 {
        g = 1.0;
    }
    for _ in 0..8 {
        g = (g + x / g) / 2.0;
    }
    g
}

/// Redondeo aproximado para no_std sin f32::round.
fn round_f32(x: f32) -> f32 {
    if x >= 0.0 {
        (x + 0.5) as i32 as f32
    } else {
        (x - 0.5) as i32 as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedding_zero() {
        let emb = Embedding::zero();
        assert!(emb.is_zero());
        assert_eq!(emb.get(0), 0.0);
        assert_eq!(emb.get(EMBEDDING_DIM - 1), 0.0);
    }

    #[test]
    fn test_embedding_from_f32() {
        let values = [1.0f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let emb = Embedding::from_f32(&values);
        assert!(!emb.is_zero());
        assert!(emb.get(0) > 0.0);
    }

    #[test]
    fn test_cosine_similarity() {
        let v1 = [1.0f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let v2 = [1.0f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let emb1 = Embedding::from_f32(&v1);
        let emb2 = Embedding::from_f32(&v2);
        let sim = emb1.cosine_similarity(&emb2);
        assert!((sim - 1.0).abs() < 0.1);
    }

    #[test]
    fn test_tokenizer() {
        let mut tokenizer = Tokenizer::new();
        let id1 = tokenizer.add_word("hola");
        let id2 = tokenizer.add_word("mundo");
        let id3 = tokenizer.add_word("hola");
        assert_eq!(id1, id3);
        assert_ne!(id1, id2);

        let tokens = tokenizer.tokenize("hola mundo hola");
        assert_eq!(tokens[0], id1);
        assert_eq!(tokens[1], id2);
        assert_eq!(tokens[2], id1);
    }

    #[test]
    fn test_embedding_model() {
        let mut model = EmbeddingModel::new();
        model.init();

        let emb1 = model.embed("archivo documento");
        let emb2 = model.embed("archivo docs");
        let emb3 = model.embed("red tcp ip");

        let sim_same = emb1.cosine_similarity(&emb2);
        let sim_diff = emb1.cosine_similarity(&emb3);

        assert!(sim_same > sim_diff);
    }

    #[test]
    fn test_text_similarity() {
        let mut model = EmbeddingModel::new();
        model.init();

        let sim1 = model.text_similarity("archivo documento", "documento archivo");
        let sim2 = model.text_similarity("archivo documento", "red tcp");

        assert!(sim1 > sim2);
    }

    #[test]
    fn test_embedding_not_zero() {
        let mut model = EmbeddingModel::new();
        model.init();
        let emb = model.embed("archivo documento");
        assert!(!emb.is_zero());
    }

    #[test]
    fn test_semantic_index() {
        let mut model = EmbeddingModel::new();
        model.init();
        let mut index = EmbeddingIndex::new();

        assert!(index.insert(10, model.embed("archivo documento")));
        assert!(index.insert(20, model.embed("red tcp ip")));
        assert_eq!(index.len(), 2);

        let mut hits = [SearchHit::empty(); 2];
        let n = index.search_text(&model, "documento archivo", &mut hits);
        assert_eq!(n, 2);
        assert_eq!(hits[0].inode_id, 10);
        assert!(hits[0].similarity >= hits[1].similarity);
    }

    #[test]
    fn test_semb_header() {
        let mut buf = [0u8; 12];
        write_u32(&mut buf, 0, SEMB_MAGIC);
        write_u16(&mut buf, 4, SEMB_VERSION);
        write_u16(&mut buf, 6, EMBEDDING_DIM as u16);
        write_u32(&mut buf, 8, 1);
        assert_eq!(read_u32(&buf, 0), Some(SEMB_MAGIC));
        assert_eq!(read_u16(&buf, 4), Some(SEMB_VERSION));
        assert_eq!(read_u16(&buf, 6), Some(EMBEDDING_DIM as u16));
        assert_eq!(read_u32(&buf, 8), Some(1));
    }

    #[test]
    fn test_semb_roundtrip() {
        let mut model = EmbeddingModel::new();
        model.init();

        let mut buf = [0u8; 65536];
        let len = model.export_semb(&mut buf).expect("export_semb");
        assert_eq!(read_u32(&buf, 0), Some(SEMB_MAGIC));
        assert_eq!(read_u16(&buf, 4), Some(SEMB_VERSION));
        assert_eq!(read_u16(&buf, 6), Some(EMBEDDING_DIM as u16));
        assert_eq!(read_u32(&buf, 8), Some(model.tokenizer.len() as u32));
        let mut loaded = EmbeddingModel::new();
        loaded.load_from_semb(&buf[..len]).expect("load_semb");

        let sim_same = loaded.text_similarity("archivo documento", "documento archivo");
        let sim_diff = loaded.text_similarity("archivo documento", "red tcp");
        assert!(sim_same > sim_diff);
    }

    #[test]
    fn test_semb_invalid_data() {
        let mut model = EmbeddingModel::new();
        model.init();

        let mut buf = [0u8; 16];
        assert!(model.load_from_semb(&buf).is_err());
    }
}
