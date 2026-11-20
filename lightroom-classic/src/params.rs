//! The Develop settings KnobDeck can turn: Lightroom Classic's `LrDevelopController` parameter
//! names (the current process version's, without the `2012` suffix), the panel each is in (for
//! `revealPanel`) and its range on a raw photo (Lightroom clamps to the photo's real range, e.g.
//! Temperature is -100..100 on a JPEG).

pub struct Param {
    pub id: &'static str,
    /// On the tile ("Shadows Hue").
    pub short: &'static str,
    pub panel: &'static str,
    pub panel_label: &'static str,
    pub min: f64,
    pub max: f64,
}

const fn p(id: &'static str, short: &'static str, panel: &'static str, panel_label: &'static str, min: f64, max: f64) -> Param {
    Param { id, short, panel, panel_label, min, max }
}

pub const PARAMS: &[Param] = &[
    // Basic
    p("Temperature", "Temp", "adjustPanel", "Basic", 2000.0, 50000.0),
    p("Tint", "Tint", "adjustPanel", "Basic", -150.0, 150.0),
    p("Exposure", "Exposure", "adjustPanel", "Basic", -5.0, 5.0),
    p("Contrast", "Contrast", "adjustPanel", "Basic", -100.0, 100.0),
    p("Highlights", "Highlights", "adjustPanel", "Basic", -100.0, 100.0),
    p("Shadows", "Shadows", "adjustPanel", "Basic", -100.0, 100.0),
    p("Whites", "Whites", "adjustPanel", "Basic", -100.0, 100.0),
    p("Blacks", "Blacks", "adjustPanel", "Basic", -100.0, 100.0),
    p("Texture", "Texture", "adjustPanel", "Basic", -100.0, 100.0),
    p("Clarity", "Clarity", "adjustPanel", "Basic", -100.0, 100.0),
    p("Dehaze", "Dehaze", "adjustPanel", "Basic", -100.0, 100.0),
    p("Vibrance", "Vibrance", "adjustPanel", "Basic", -100.0, 100.0),
    p("Saturation", "Saturation", "adjustPanel", "Basic", -100.0, 100.0),
    // Tone Curve
    p("ParametricHighlights", "Highlights", "tonePanel", "Tone Curve", -100.0, 100.0),
    p("ParametricLights", "Lights", "tonePanel", "Tone Curve", -100.0, 100.0),
    p("ParametricDarks", "Darks", "tonePanel", "Tone Curve", -100.0, 100.0),
    p("ParametricShadows", "Shadows", "tonePanel", "Tone Curve", -100.0, 100.0),
    p("ParametricShadowSplit", "Shadow Split", "tonePanel", "Tone Curve", 10.0, 70.0),
    p("ParametricMidtoneSplit", "Midtone Split", "tonePanel", "Tone Curve", 20.0, 80.0),
    p("ParametricHighlightSplit", "Highlight Split", "tonePanel", "Tone Curve", 30.0, 90.0),
    // HSL
    p("HueAdjustmentRed", "Red Hue", "mixerPanel", "HSL", -100.0, 100.0),
    p("HueAdjustmentOrange", "Orange Hue", "mixerPanel", "HSL", -100.0, 100.0),
    p("HueAdjustmentYellow", "Yellow Hue", "mixerPanel", "HSL", -100.0, 100.0),
    p("HueAdjustmentGreen", "Green Hue", "mixerPanel", "HSL", -100.0, 100.0),
    p("HueAdjustmentAqua", "Aqua Hue", "mixerPanel", "HSL", -100.0, 100.0),
    p("HueAdjustmentBlue", "Blue Hue", "mixerPanel", "HSL", -100.0, 100.0),
    p("HueAdjustmentPurple", "Purple Hue", "mixerPanel", "HSL", -100.0, 100.0),
    p("HueAdjustmentMagenta", "Magenta Hue", "mixerPanel", "HSL", -100.0, 100.0),
    p("SaturationAdjustmentRed", "Red Saturation", "mixerPanel", "HSL", -100.0, 100.0),
    p("SaturationAdjustmentOrange", "Orange Saturation", "mixerPanel", "HSL", -100.0, 100.0),
    p("SaturationAdjustmentYellow", "Yellow Saturation", "mixerPanel", "HSL", -100.0, 100.0),
    p("SaturationAdjustmentGreen", "Green Saturation", "mixerPanel", "HSL", -100.0, 100.0),
    p("SaturationAdjustmentAqua", "Aqua Saturation", "mixerPanel", "HSL", -100.0, 100.0),
    p("SaturationAdjustmentBlue", "Blue Saturation", "mixerPanel", "HSL", -100.0, 100.0),
    p("SaturationAdjustmentPurple", "Purple Saturation", "mixerPanel", "HSL", -100.0, 100.0),
    p("SaturationAdjustmentMagenta", "Magenta Saturation", "mixerPanel", "HSL", -100.0, 100.0),
    p("LuminanceAdjustmentRed", "Red Luminance", "mixerPanel", "HSL", -100.0, 100.0),
    p("LuminanceAdjustmentOrange", "Orange Luminance", "mixerPanel", "HSL", -100.0, 100.0),
    p("LuminanceAdjustmentYellow", "Yellow Luminance", "mixerPanel", "HSL", -100.0, 100.0),
    p("LuminanceAdjustmentGreen", "Green Luminance", "mixerPanel", "HSL", -100.0, 100.0),
    p("LuminanceAdjustmentAqua", "Aqua Luminance", "mixerPanel", "HSL", -100.0, 100.0),
    p("LuminanceAdjustmentBlue", "Blue Luminance", "mixerPanel", "HSL", -100.0, 100.0),
    p("LuminanceAdjustmentPurple", "Purple Luminance", "mixerPanel", "HSL", -100.0, 100.0),
    p("LuminanceAdjustmentMagenta", "Magenta Luminance", "mixerPanel", "HSL", -100.0, 100.0),
    // Color Grading
    p("SplitToningShadowHue", "Shadows Hue", "colorGradingPanel", "Color Grading", 0.0, 360.0),
    p("SplitToningShadowSaturation", "Shadows Sat", "colorGradingPanel", "Color Grading", 0.0, 100.0),
    p("ColorGradeShadowLum", "Shadows Lum", "colorGradingPanel", "Color Grading", -100.0, 100.0),
    p("ColorGradeMidtoneHue", "Midtones Hue", "colorGradingPanel", "Color Grading", 0.0, 360.0),
    p("ColorGradeMidtoneSat", "Midtones Sat", "colorGradingPanel", "Color Grading", 0.0, 100.0),
    p("ColorGradeMidtoneLum", "Midtones Lum", "colorGradingPanel", "Color Grading", -100.0, 100.0),
    p("SplitToningHighlightHue", "Highlights Hue", "colorGradingPanel", "Color Grading", 0.0, 360.0),
    p("SplitToningHighlightSaturation", "Highlights Sat", "colorGradingPanel", "Color Grading", 0.0, 100.0),
    p("ColorGradeHighlightLum", "Highlights Lum", "colorGradingPanel", "Color Grading", -100.0, 100.0),
    p("ColorGradeGlobalHue", "Global Hue", "colorGradingPanel", "Color Grading", 0.0, 360.0),
    p("ColorGradeGlobalSat", "Global Sat", "colorGradingPanel", "Color Grading", 0.0, 100.0),
    p("ColorGradeGlobalLum", "Global Lum", "colorGradingPanel", "Color Grading", -100.0, 100.0),
    p("ColorGradeBlending", "Blending", "colorGradingPanel", "Color Grading", 0.0, 100.0),
    p("SplitToningBalance", "Balance", "colorGradingPanel", "Color Grading", -100.0, 100.0),
    // Detail
    p("Sharpness", "Sharpening", "detailPanel", "Detail", 0.0, 150.0),
    p("SharpenRadius", "Sharpen Radius", "detailPanel", "Detail", 0.5, 3.0),
    p("SharpenDetail", "Sharpen Detail", "detailPanel", "Detail", 0.0, 100.0),
    p("SharpenEdgeMasking", "Masking", "detailPanel", "Detail", 0.0, 100.0),
    p("LuminanceSmoothing", "Noise Reduction", "detailPanel", "Detail", 0.0, 100.0),
    p("LuminanceNoiseReductionDetail", "NR Detail", "detailPanel", "Detail", 0.0, 100.0),
    p("LuminanceNoiseReductionContrast", "NR Contrast", "detailPanel", "Detail", 0.0, 100.0),
    p("ColorNoiseReduction", "Color NR", "detailPanel", "Detail", 0.0, 100.0),
    p("ColorNoiseReductionDetail", "Color NR Detail", "detailPanel", "Detail", 0.0, 100.0),
    p("ColorNoiseReductionSmoothness", "Color NR Smooth", "detailPanel", "Detail", 0.0, 100.0),
    // Lens Corrections
    p("LensProfileDistortionScale", "Profile Distortion", "lensCorrectionsPanel", "Lens Corrections", 0.0, 200.0),
    p("LensProfileVignettingScale", "Profile Vignetting", "lensCorrectionsPanel", "Lens Corrections", 0.0, 200.0),
    p("LensManualDistortionAmount", "Distortion", "lensCorrectionsPanel", "Lens Corrections", -100.0, 100.0),
    p("VignetteAmount", "Lens Vignetting", "lensCorrectionsPanel", "Lens Corrections", -100.0, 100.0),
    p("VignetteMidpoint", "Lens Vig. Midpoint", "lensCorrectionsPanel", "Lens Corrections", 0.0, 100.0),
    p("DefringePurpleAmount", "Purple Amount", "lensCorrectionsPanel", "Lens Corrections", 0.0, 20.0),
    p("DefringePurpleHueLo", "Purple Hue Low", "lensCorrectionsPanel", "Lens Corrections", 0.0, 100.0),
    p("DefringePurpleHueHi", "Purple Hue High", "lensCorrectionsPanel", "Lens Corrections", 0.0, 100.0),
    p("DefringeGreenAmount", "Green Amount", "lensCorrectionsPanel", "Lens Corrections", 0.0, 20.0),
    p("DefringeGreenHueLo", "Green Hue Low", "lensCorrectionsPanel", "Lens Corrections", 0.0, 100.0),
    p("DefringeGreenHueHi", "Green Hue High", "lensCorrectionsPanel", "Lens Corrections", 0.0, 100.0),
    // Transform
    p("PerspectiveVertical", "Vertical", "transformPanel", "Transform", -100.0, 100.0),
    p("PerspectiveHorizontal", "Horizontal", "transformPanel", "Transform", -100.0, 100.0),
    p("PerspectiveRotate", "Rotate", "transformPanel", "Transform", -10.0, 10.0),
    p("PerspectiveAspect", "Aspect", "transformPanel", "Transform", -100.0, 100.0),
    p("PerspectiveScale", "Scale", "transformPanel", "Transform", 50.0, 150.0),
    p("PerspectiveX", "Offset X", "transformPanel", "Transform", -100.0, 100.0),
    p("PerspectiveY", "Offset Y", "transformPanel", "Transform", -100.0, 100.0),
    // Effects
    p("PostCropVignetteAmount", "Vignette", "effectsPanel", "Effects", -100.0, 100.0),
    p("PostCropVignetteMidpoint", "Vignette Midpoint", "effectsPanel", "Effects", 0.0, 100.0),
    p("PostCropVignetteRoundness", "Vignette Roundness", "effectsPanel", "Effects", -100.0, 100.0),
    p("PostCropVignetteFeather", "Vignette Feather", "effectsPanel", "Effects", 0.0, 100.0),
    p("PostCropVignetteHighlightContrast", "Vignette Highlights", "effectsPanel", "Effects", 0.0, 100.0),
    p("GrainAmount", "Grain", "effectsPanel", "Effects", 0.0, 100.0),
    p("GrainSize", "Grain Size", "effectsPanel", "Effects", 0.0, 100.0),
    p("GrainFrequency", "Grain Roughness", "effectsPanel", "Effects", 0.0, 100.0),
    // Calibration
    p("ShadowTint", "Shadows Tint", "calibratePanel", "Calibration", -100.0, 100.0),
    p("RedHue", "Red Primary Hue", "calibratePanel", "Calibration", -100.0, 100.0),
    p("RedSaturation", "Red Primary Sat", "calibratePanel", "Calibration", -100.0, 100.0),
    p("GreenHue", "Green Primary Hue", "calibratePanel", "Calibration", -100.0, 100.0),
    p("GreenSaturation", "Green Primary Sat", "calibratePanel", "Calibration", -100.0, 100.0),
    p("BlueHue", "Blue Primary Hue", "calibratePanel", "Calibration", -100.0, 100.0),
    p("BlueSaturation", "Blue Primary Sat", "calibratePanel", "Calibration", -100.0, 100.0),
];

pub fn find(id: &str) -> Option<&'static Param> {
    PARAMS.iter().find(|p| p.id == id)
}

/// Picker label: "Color Grading: Shadows Hue".
pub fn label(p: &Param) -> String {
    format!("{}: {}", p.panel_label, p.short)
}

/// Develop panels for Show Panel, in Lightroom's order.
pub const PANELS: &[(&str, &str)] = &[
    ("adjustPanel", "Basic"),
    ("tonePanel", "Tone Curve"),
    ("mixerPanel", "HSL / Color"),
    ("colorGradingPanel", "Color Grading"),
    ("detailPanel", "Detail"),
    ("lensCorrectionsPanel", "Lens Corrections"),
    ("transformPanel", "Transform"),
    ("effectsPanel", "Effects"),
    ("calibratePanel", "Calibration"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_ranges_sane() {
        let mut seen = std::collections::HashSet::new();
        for p in PARAMS {
            assert!(seen.insert(p.id), "{} twice", p.id);
            assert!(p.min < p.max, "{}", p.id);
            assert!(p.id.chars().all(|c| c.is_ascii_alphanumeric()), "{}", p.id);
            assert!(PANELS.iter().any(|(id, _)| *id == p.panel), "{} panel {}", p.id, p.panel);
        }
        assert_eq!(label(find("SplitToningShadowHue").unwrap()), "Color Grading: Shadows Hue");
    }
}
