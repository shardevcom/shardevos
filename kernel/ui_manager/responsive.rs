// Responsive Design — Diseño Responsivo para UI Manager de shardevos.
//
// Proporciona:
// - Breakpoints para distintos dispositivos (mobile, tablet, desktop, tv)
// - Detección de categoría de dispositivo según resolución
// - Gestión de orientación (portrait/landscape)
// - Sistema de grid fluido y escalado de UI
// - Reglas de estilo adaptativas por dispositivo


use core::sync::atomic::{AtomicU64, Ordering};

/// Breakpoints de ancho de pantalla (en píxeles).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum Breakpoint {
    /// Móviles pequeños (< 480px)
    Xs = 0,
    /// Móviles (480px - 767px)
    Sm = 480,
    /// Tablets (768px - 1023px)
    Md = 768,
    /// Escritorio pequeño (1024px - 1279px)
    Lg = 1024,
    /// Escritorio (1280px - 1919px)
    Xl = 1280,
    /// Escritorio grande / TV (1920px+)
    Xxl = 1920,
}

/// Categoría de dispositivo según resolución.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DeviceCategory {
    Mobile = 0,
    Tablet = 1,
    Desktop = 2,
    Tv = 3,
}

/// Orientación de la pantalla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Orientation {
    Portrait = 0,
    Landscape = 1,
}

/// Configuración de diseño responsivo para un dispositivo.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ResponsiveConfig {
    pub category: DeviceCategory,
    pub orientation: Orientation,
    pub scale_factor: f32,       // Escala de UI (1.0 = normal)
    pub font_scale: f32,         // Escala de fuente
    pub padding_scale: f32,      // Escala de márgenes
    pub gutter_size: u16,        // Espacio entre columnas
    pub max_columns: u8,         // Columnas máximas en grid
    pub sidebar_collapsed: bool, // Si el sidebar está colapsado por defecto
    pub touch_targets: bool,     // Si usar targets táctiles grandes
}

/// Reglas de breakpoint.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct BreakpointRule {
    pub min_width: u16,
    pub category: DeviceCategory,
    pub orientation: Orientation,
    pub scale_factor: f32,
    pub font_scale: f32,
    pub padding_scale: f32,
    pub gutter_size: u16,
    pub max_columns: u8,
    pub sidebar_collapsed: bool,
    pub touch_targets: bool,
}

/// Gestor de diseño responsivo.
pub struct ResponsiveManager {
    current_config: AtomicU64,  // Empaquetado: category(8) + orientation(8) + escala(16)
    screen_width: AtomicU64,
    screen_height: AtomicU64,
    breakpoint_rules: [BreakpointRule; 6],
    rules_count: usize,
    initialized: AtomicU64,
}

impl ResponsiveManager {
    pub const fn new() -> Self {
        Self {
            current_config: AtomicU64::new(0),
            screen_width: AtomicU64::new(0),
            screen_height: AtomicU64::new(0),
            breakpoint_rules: [
                // XS - Móviles pequeños
                BreakpointRule {
                    min_width: 0,
                    category: DeviceCategory::Mobile,
                    orientation: Orientation::Portrait,
                    scale_factor: 0.8,
                    font_scale: 0.9,
                    padding_scale: 0.7,
                    gutter_size: 8,
                    max_columns: 1,
                    sidebar_collapsed: true,
                    touch_targets: true,
                },
                // SM - Móviles
                BreakpointRule {
                    min_width: 480,
                    category: DeviceCategory::Mobile,
                    orientation: Orientation::Portrait,
                    scale_factor: 0.9,
                    font_scale: 0.95,
                    padding_scale: 0.8,
                    gutter_size: 12,
                    max_columns: 2,
                    sidebar_collapsed: true,
                    touch_targets: true,
                },
                // MD - Tablets
                BreakpointRule {
                    min_width: 768,
                    category: DeviceCategory::Tablet,
                    orientation: Orientation::Portrait,
                    scale_factor: 1.0,
                    font_scale: 1.0,
                    padding_scale: 1.0,
                    gutter_size: 16,
                    max_columns: 3,
                    sidebar_collapsed: false,
                    touch_targets: true,
                },
                // LG - Escritorio pequeño
                BreakpointRule {
                    min_width: 1024,
                    category: DeviceCategory::Desktop,
                    orientation: Orientation::Landscape,
                    scale_factor: 1.1,
                    font_scale: 1.05,
                    padding_scale: 1.1,
                    gutter_size: 20,
                    max_columns: 4,
                    sidebar_collapsed: false,
                    touch_targets: false,
                },
                // XL - Escritorio
                BreakpointRule {
                    min_width: 1280,
                    category: DeviceCategory::Desktop,
                    orientation: Orientation::Landscape,
                    scale_factor: 1.2,
                    font_scale: 1.1,
                    padding_scale: 1.2,
                    gutter_size: 24,
                    max_columns: 6,
                    sidebar_collapsed: false,
                    touch_targets: false,
                },
                // XXL - Escritorio grande / TV
                BreakpointRule {
                    min_width: 1920,
                    category: DeviceCategory::Tv,
                    orientation: Orientation::Landscape,
                    scale_factor: 1.5,
                    font_scale: 1.3,
                    padding_scale: 1.5,
                    gutter_size: 32,
                    max_columns: 8,
                    sidebar_collapsed: false,
                    touch_targets: false,
                },
            ],
            rules_count: 6,
            initialized: AtomicU64::new(0),
        }
    }

    pub unsafe fn init(&self) {
        self.initialized.store(1, Ordering::Release);
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire) != 0
    }

    /// Establece las dimensiones de pantalla y calcula la configuración responsiva.
    pub fn set_screen_size(&self, width: u16, height: u16) {
        if !self.is_initialized() {
            return;
        }

        self.screen_width.store(width as u64, Ordering::Release);
        self.screen_height.store(height as u64, Ordering::Release);

        let orientation = if width >= height {
            Orientation::Landscape
        } else {
            Orientation::Portrait
        };

        // Encontrar la regla adecuada basada en el ancho
        let mut best_rule = &self.breakpoint_rules[0];
        for i in 0..self.rules_count {
            let rule = &self.breakpoint_rules[i];
            if width >= rule.min_width {
                best_rule = rule;
            } else {
                break;
            }
        }

        let config = ResponsiveConfig {
            category: best_rule.category,
            orientation,
            scale_factor: best_rule.scale_factor,
            font_scale: best_rule.font_scale,
            padding_scale: best_rule.padding_scale,
            gutter_size: best_rule.gutter_size,
            max_columns: best_rule.max_columns,
            sidebar_collapsed: best_rule.sidebar_collapsed,
            touch_targets: best_rule.touch_targets,
        };

        // Empaquetar configuración en un u64 para almacenamiento atómico
        let packed = self.pack_config(&config);
        self.current_config.store(packed, Ordering::Release);
    }

    /// Obtiene la configuración responsiva actual.
    pub fn current_config(&self) -> ResponsiveConfig {
        let packed = self.current_config.load(Ordering::Acquire);
        self.unpack_config(packed)
    }

    /// Devuelve la categoría de dispositivo actual.
    pub fn current_category(&self) -> DeviceCategory {
        self.current_config().category
    }

    /// Devuelve la orientación actual.
    pub fn current_orientation(&self) -> Orientation {
        self.current_config().orientation
    }

    /// Devuelve el ancho de pantalla actual.
    pub fn screen_width(&self) -> u16 {
        self.screen_width.load(Ordering::Acquire) as u16
    }

    /// Devuelve el alto de pantalla actual.
    pub fn screen_height(&self) -> u16 {
        self.screen_height.load(Ordering::Acquire) as u16
    }

    /// Empaqueta ResponsiveConfig en u64 para almacenamiento atómico.
    fn pack_config(&self, config: &ResponsiveConfig) -> u64 {
        let mut packed: u64 = 0;
        packed |= (config.category as u64) & 0xFF;
        packed |= ((config.orientation as u64) & 0xFF) << 8;
        packed |= (((config.scale_factor * 100.0) as u16) as u64) << 16;
        packed |= (((config.font_scale * 100.0) as u16) as u64) << 32;
        packed |= (((config.padding_scale * 100.0) as u16) as u64) << 48;
        packed
    }

    /// Desempaqueta u64 a ResponsiveConfig.
    fn unpack_config(&self, packed: u64) -> ResponsiveConfig {
        let category = DeviceCategory::from_u8((packed & 0xFF) as u8);
        let orientation = Orientation::from_u8(((packed >> 8) & 0xFF) as u8);
        let scale_factor = ((packed >> 16) & 0xFFFF) as u16 as f32 / 100.0;
        let font_scale = ((packed >> 32) & 0xFFFF) as u16 as f32 / 100.0;
        let padding_scale = ((packed >> 48) & 0xFFFF) as u16 as f32 / 100.0;

    // Calcular gutter_size y max_columns basado en la categoría
    let (gutter_size, max_columns, sidebar_collapsed, touch_targets) =
        Self::defaults_for_category(category);

        ResponsiveConfig {
            category,
            orientation,
            scale_factor,
            font_scale,
            padding_scale,
            gutter_size,
            max_columns,
            sidebar_collapsed,
            touch_targets,
        }
    }

    /// Valores por defecto para una categoría de dispositivo.
    fn defaults_for_category(category: DeviceCategory) -> (u16, u8, bool, bool) {
        match category {
            DeviceCategory::Mobile => (8, 1, true, true),
            DeviceCategory::Tablet => (16, 3, false, true),
            DeviceCategory::Desktop => (20, 4, false, false),
            DeviceCategory::Tv => (32, 8, false, false),
        }
    }
}

impl DeviceCategory {
    pub fn from_u8(val: u8) -> Self {
        match val {
            0 => DeviceCategory::Mobile,
            1 => DeviceCategory::Tablet,
            2 => DeviceCategory::Desktop,
            _ => DeviceCategory::Tv,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            DeviceCategory::Mobile => "mobile",
            DeviceCategory::Tablet => "tablet",
            DeviceCategory::Desktop => "desktop",
            DeviceCategory::Tv => "tv",
        }
    }

    pub fn min_width(&self) -> u16 {
        match self {
            DeviceCategory::Mobile => 0,
            DeviceCategory::Tablet => 768,
            DeviceCategory::Desktop => 1024,
            DeviceCategory::Tv => 1920,
        }
    }
}

impl Orientation {
    pub fn from_u8(val: u8) -> Self {
        match val {
            0 => Orientation::Portrait,
            _ => Orientation::Landscape,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Orientation::Portrait => "portrait",
            Orientation::Landscape => "landscape",
        }
    }
}

/// Instancia global del gestor responsivo.
static mut RESPONSIVE_MANAGER: ResponsiveManager = ResponsiveManager::new();

/// Devuelve una referencia mutable al gestor responsivo global.
#[allow(static_mut_refs)]
pub fn global_responsive_manager() -> &'static ResponsiveManager {
    unsafe { &RESPONSIVE_MANAGER }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breakpoints() {
        assert_eq!(Breakpoint::Xs as u16, 0);
        assert_eq!(Breakpoint::Sm as u16, 480);
        assert_eq!(Breakpoint::Md as u16, 768);
        assert_eq!(Breakpoint::Lg as u16, 1024);
        assert_eq!(Breakpoint::Xl as u16, 1280);
        assert_eq!(Breakpoint::Xxl as u16, 1920);
    }

    #[test]
    fn device_categories() {
        assert_eq!(DeviceCategory::Mobile as u8, 0);
        assert_eq!(DeviceCategory::Tablet as u8, 1);
        assert_eq!(DeviceCategory::Desktop as u8, 2);
        assert_eq!(DeviceCategory::Tv as u8, 3);

        assert_eq!(DeviceCategory::Mobile.name(), "mobile");
        assert_eq!(DeviceCategory::Tablet.name(), "tablet");
        assert_eq!(DeviceCategory::Desktop.name(), "desktop");
        assert_eq!(DeviceCategory::Tv.name(), "tv");

        assert_eq!(DeviceCategory::Mobile.min_width(), 0);
        assert_eq!(DeviceCategory::Tablet.min_width(), 768);
        assert_eq!(DeviceCategory::Desktop.min_width(), 1024);
        assert_eq!(DeviceCategory::Tv.min_width(), 1920);
    }

    #[test]
    fn orientations() {
        assert_eq!(Orientation::Portrait as u8, 0);
        assert_eq!(Orientation::Landscape as u8, 1);
        assert_eq!(Orientation::Portrait.name(), "portrait");
        assert_eq!(Orientation::Landscape.name(), "landscape");
    }

    #[test]
    fn responsive_manager_new() {
        let mgr = ResponsiveManager::new();
        assert!(!mgr.is_initialized());
    }

    #[test]
    fn responsive_manager_init() {
        let mgr = ResponsiveManager::new();
        unsafe { mgr.init(); }
        assert!(mgr.is_initialized());
    }

    #[test]
    fn set_screen_size_mobile() {
        let mgr = ResponsiveManager::new();
        unsafe { mgr.init(); }

        mgr.set_screen_size(360, 640); // Móvil pequeño
        let config = mgr.current_config();
        assert_eq!(config.category, DeviceCategory::Mobile);
        assert_eq!(config.orientation, Orientation::Portrait);
        assert_eq!(config.max_columns, 1);
        assert!(config.touch_targets);
        assert!(config.sidebar_collapsed);
    }

    #[test]
    fn set_screen_size_tablet() {
        let mgr = ResponsiveManager::new();
        unsafe { mgr.init(); }

        mgr.set_screen_size(768, 1024); // Tablet
        let config = mgr.current_config();
        assert_eq!(config.category, DeviceCategory::Tablet);
        assert_eq!(config.max_columns, 3);
        assert!(config.touch_targets);
    }

    #[test]
    fn set_screen_size_desktop() {
        let mgr = ResponsiveManager::new();
        unsafe { mgr.init(); }

        mgr.set_screen_size(1600, 900); // Escritorio
        let config = mgr.current_config();
        assert_eq!(config.category, DeviceCategory::Desktop);
        assert_eq!(config.orientation, Orientation::Landscape);
        assert_eq!(config.max_columns, 4);
        assert!(!config.touch_targets);
    }

    #[test]
    fn set_screen_size_tv() {
        let mgr = ResponsiveManager::new();
        unsafe { mgr.init(); }

        mgr.set_screen_size(1920, 1080); // TV
        let config = mgr.current_config();
        assert_eq!(config.category, DeviceCategory::Tv);
        assert_eq!(config.max_columns, 8);
        assert!(!config.touch_targets);
    }

    #[test]
    fn scale_factors() {
        let mgr = ResponsiveManager::new();
        unsafe { mgr.init(); }

        mgr.set_screen_size(360, 640);
        let config = mgr.current_config();
        assert!((config.scale_factor - 0.8).abs() < 0.01);
        assert!((config.font_scale - 0.9).abs() < 0.01);

        mgr.set_screen_size(1920, 1080);
        let config = mgr.current_config();
        assert!((config.scale_factor - 1.5).abs() < 0.01);
        assert!((config.font_scale - 1.3).abs() < 0.01);
    }
}
