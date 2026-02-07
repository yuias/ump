//! DirectWrite text rendering: font setup, cell metrics measurement.
#![cfg(feature = "d2d")]

use std::collections::HashMap;

use windows::core::{Interface, HSTRING};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_SIMULATIONS_NONE,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_BOLD,
    DWRITE_FONT_WEIGHT_REGULAR, DWRITE_TEXT_METRICS, IDWriteFactory, IDWriteFactory3,
    IDWriteFontCollection, IDWriteFontCollection1, IDWriteTextFormat,
};

use crate::renderer::RenderError;

/// Manages DirectWrite text formatting and cell metrics.
pub struct TextRenderer {
    pub factory: IDWriteFactory,
    pub format_regular: IDWriteTextFormat,
    pub format_bold: IDWriteTextFormat,
    pub cell_width: f32,
    pub cell_height: f32,
    /// Cached IDWriteTextFormat by size (key = (size * 10) as u32).
    format_cache: HashMap<u32, IDWriteTextFormat>,
    font_family: HSTRING,
    locale: HSTRING,
    collection: Option<IDWriteFontCollection1>,
}

impl TextRenderer {
    /// Create a TextRenderer using the specified font family and size.
    pub fn new(font_family: &str, font_size: f32) -> Result<Self, RenderError> {
        let factory: IDWriteFactory = unsafe {
            DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).map_err(|e| {
                RenderError::PlatformError(format!("DWriteCreateFactory failed: {}", e))
            })?
        };

        let family = HSTRING::from(font_family);
        let locale = HSTRING::from("en-us");

        let format_regular = unsafe {
            factory
                .CreateTextFormat(
                    &family,
                    None,
                    DWRITE_FONT_WEIGHT_REGULAR,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    font_size,
                    &locale,
                )
                .map_err(|e| {
                    RenderError::PlatformError(format!(
                        "CreateTextFormat (regular) failed: {}",
                        e
                    ))
                })?
        };

        let format_bold = unsafe {
            factory
                .CreateTextFormat(
                    &family,
                    None,
                    DWRITE_FONT_WEIGHT_BOLD,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    font_size,
                    &locale,
                )
                .map_err(|e| {
                    RenderError::PlatformError(format!("CreateTextFormat (bold) failed: {}", e))
                })?
        };

        // Measure cell size using "M" character
        let (cell_width, cell_height) = Self::measure_cell(&factory, &format_regular, font_size)?;

        log_info!("Font: family={}, size={}, cell={}x{}", font_family, font_size, cell_width, cell_height);

        Ok(TextRenderer {
            factory,
            format_regular,
            format_bold,
            cell_width,
            cell_height,
            format_cache: HashMap::new(),
            font_family: family,
            locale,
            collection: None,
        })
    }

    /// Get or create an IDWriteTextFormat for the given font size.
    pub fn get_format(&mut self, size: f32) -> &IDWriteTextFormat {
        let key = (size * 10.0) as u32;
        if !self.format_cache.contains_key(&key) {
            let collection_param = self.collection.as_ref().map(|c| {
                let base: IDWriteFontCollection = c.cast().expect("cast to IDWriteFontCollection");
                base
            });
            let format = unsafe {
                self.factory
                    .CreateTextFormat(
                        &self.font_family,
                        collection_param.as_ref(),
                        DWRITE_FONT_WEIGHT_REGULAR,
                        DWRITE_FONT_STYLE_NORMAL,
                        DWRITE_FONT_STRETCH_NORMAL,
                        size,
                        &self.locale,
                    )
                    .expect("CreateTextFormat failed for cached size")
            };
            self.format_cache.insert(key, format);
        }
        &self.format_cache[&key]
    }

    /// Measure the dimensions of a single monospace cell.
    fn measure_cell(
        factory: &IDWriteFactory,
        format: &IDWriteTextFormat,
        _font_size: f32,
    ) -> Result<(f32, f32), RenderError> {
        let text = HSTRING::from("M");
        let layout = unsafe {
            factory
                .CreateTextLayout(text.as_wide(), format, 1000.0, 1000.0)
                .map_err(|e| {
                    RenderError::PlatformError(format!("CreateTextLayout failed: {}", e))
                })?
        };

        let mut metrics = DWRITE_TEXT_METRICS::default();
        unsafe {
            layout.GetMetrics(&mut metrics).map_err(|e| {
                RenderError::PlatformError(format!("GetMetrics failed: {}", e))
            })?;
        }

        Ok((metrics.width, metrics.height))
    }

    /// Create a TextRenderer with a custom font file (.ttf/.otf/.ttc).
    /// Uses IDWriteFactory3 + FontSetBuilder to create a private font collection.
    pub fn with_font_file(font_path: &str, font_size: f32) -> Result<Self, RenderError> {
        let factory: IDWriteFactory = unsafe {
            DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).map_err(|e| {
                RenderError::PlatformError(format!("DWriteCreateFactory failed: {}", e))
            })?
        };

        // Cast to IDWriteFactory3 for font set builder API
        let factory3: IDWriteFactory3 = factory.cast().map_err(|e| {
            RenderError::PlatformError(format!(
                "IDWriteFactory3 cast failed (requires Windows 10+): {}",
                e
            ))
        })?;

        // Create font file reference
        let path_hstring = HSTRING::from(font_path);
        let font_file = unsafe {
            factory3
                .CreateFontFileReference(&path_hstring, None)
                .map_err(|e| {
                    RenderError::PlatformError(format!(
                        "CreateFontFileReference failed for '{}': {}",
                        font_path, e
                    ))
                })?
        };

        // Build font set → font collection
        let builder = unsafe {
            factory3.CreateFontSetBuilder().map_err(|e| {
                RenderError::PlatformError(format!("CreateFontSetBuilder failed: {}", e))
            })?
        };

        let face_ref = unsafe {
            factory3
                .CreateFontFaceReference(&font_file, 0, DWRITE_FONT_SIMULATIONS_NONE)
                .map_err(|e| {
                    RenderError::PlatformError(format!(
                        "CreateFontFaceReference failed: {}",
                        e
                    ))
                })?
        };

        unsafe {
            builder.AddFontFaceReference2(&face_ref).map_err(|e| {
                RenderError::PlatformError(format!("AddFontFaceReference failed: {}", e))
            })?;
        }

        let font_set = unsafe {
            builder.CreateFontSet().map_err(|e| {
                RenderError::PlatformError(format!("CreateFontSet failed: {}", e))
            })?
        };

        let collection: IDWriteFontCollection1 = unsafe {
            factory3
                .CreateFontCollectionFromFontSet(&font_set)
                .map_err(|e| {
                    RenderError::PlatformError(format!(
                        "CreateFontCollectionFromFontSet failed: {}",
                        e
                    ))
                })?
        };

        // Extract family name from the collection (index 0)
        let family_name = unsafe {
            let family = collection.GetFontFamily(0).map_err(|e| {
                RenderError::PlatformError(format!("GetFontFamily(0) failed: {}", e))
            })?;
            let names = family.GetFamilyNames().map_err(|e| {
                RenderError::PlatformError(format!("GetFamilyNames failed: {}", e))
            })?;
            let len = names.GetStringLength(0).map_err(|e| {
                RenderError::PlatformError(format!("GetStringLength failed: {}", e))
            })?;
            let mut buf = vec![0u16; (len + 1) as usize];
            names.GetString(0, &mut buf).map_err(|e| {
                RenderError::PlatformError(format!("GetString failed: {}", e))
            })?;
            // Trim null terminator
            if let Some(pos) = buf.iter().position(|&c| c == 0) {
                buf.truncate(pos);
            }
            String::from_utf16_lossy(&buf)
        };

        let family_hstring = HSTRING::from(&family_name);
        let locale = HSTRING::from("en-us");

        // Cast collection to base interface for CreateTextFormat
        let collection_base: IDWriteFontCollection = collection.cast().map_err(|e| {
            RenderError::PlatformError(format!("IDWriteFontCollection cast failed: {}", e))
        })?;

        let format_regular = unsafe {
            factory
                .CreateTextFormat(
                    &family_hstring,
                    Some(&collection_base),
                    DWRITE_FONT_WEIGHT_REGULAR,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    font_size,
                    &locale,
                )
                .map_err(|e| {
                    RenderError::PlatformError(format!(
                        "CreateTextFormat (regular) failed: {}",
                        e
                    ))
                })?
        };

        let format_bold = unsafe {
            factory
                .CreateTextFormat(
                    &family_hstring,
                    Some(&collection_base),
                    DWRITE_FONT_WEIGHT_BOLD,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    font_size,
                    &locale,
                )
                .map_err(|e| {
                    RenderError::PlatformError(format!(
                        "CreateTextFormat (bold) failed: {}",
                        e
                    ))
                })?
        };

        let (cell_width, cell_height) = Self::measure_cell(&factory, &format_regular, font_size)?;

        log_info!(
            "Custom font: path={}, family={}, size={}, cell={}x{}",
            font_path,
            family_name,
            font_size,
            cell_width,
            cell_height
        );

        Ok(TextRenderer {
            factory,
            format_regular,
            format_bold,
            cell_width,
            cell_height,
            format_cache: HashMap::new(),
            font_family: family_hstring,
            locale,
            collection: Some(collection),
        })
    }
}
