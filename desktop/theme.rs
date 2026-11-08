//! Huda's visual tokens: the system font at the macOS type scale, and one semantic palette for
//! each appearance. Appearance follows the system, so there is no setting for it. Colors come
//! from the icon: the deep blue field for actions and the amber crossbar for relations.
use eframe::egui::{
    self, epaint::text::VariationCoords, Color32, CornerRadius, FontData, FontDefinitions,
    FontFamily, FontId, FontTweak, Stroke, TextStyle, Theme, ThemePreference, Visuals,
};
use std::sync::Arc;

/// What each color is for. Nothing draws with a literal color.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub window: Color32,
    pub sidebar: Color32,
    /// A grouped section on the window.
    pub surface: Color32,
    pub control: Color32,
    pub control_hover: Color32,
    pub separator: Color32,
    pub text: Color32,
    pub secondary: Color32,
    /// Placeholders and the quietest supporting text.
    pub tertiary: Color32,
    /// The fill of a primary button.
    pub accent: Color32,
    pub on_accent: Color32,
    pub accent_text: Color32,
    /// The fill of the selected sidebar row and of selected text.
    pub selection: Color32,
    /// The crossbar of the icon: only ever a relation or the current place.
    pub crossbar: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub on_danger: Color32,
    pub danger_fill: Color32,
}

const fn hex(value: u32) -> Color32 {
    Color32::from_rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

pub const LIGHT: Palette = Palette {
    window: hex(0xF5F5F7),
    sidebar: hex(0xE8E8ED),
    surface: hex(0xFFFFFF),
    control: hex(0xFFFFFF),
    control_hover: hex(0xEDEDF2),
    separator: hex(0xD8D8DE),
    text: hex(0x1D1D1F),
    secondary: hex(0x55555C),
    tertiary: hex(0x68686F),
    accent: hex(0x0F5BD7),
    on_accent: hex(0xFFFFFF),
    accent_text: hex(0x0B57D0),
    selection: hex(0xD3E0F8),
    crossbar: hex(0xB26A00),
    success: hex(0x15702F),
    warning: hex(0x8A5300),
    danger: hex(0xB3261E),
    on_danger: hex(0xFFFFFF),
    danger_fill: hex(0xC42B1C),
};

pub const DARK: Palette = Palette {
    window: hex(0x1C1C1E),
    sidebar: hex(0x262628),
    surface: hex(0x2A2A2D),
    control: hex(0x3A3A3D),
    control_hover: hex(0x48484C),
    separator: hex(0x3E3E42),
    text: hex(0xF5F5F7),
    secondary: hex(0xB4B4BA),
    tertiary: hex(0x9C9CA3),
    accent: hex(0x1F6FEB),
    on_accent: hex(0xFFFFFF),
    accent_text: hex(0x8AB8FF),
    selection: hex(0x2C3F63),
    crossbar: hex(0xFFB340),
    success: hex(0x56D364),
    warning: hex(0xFFB340),
    danger: hex(0xFF8A80),
    on_danger: hex(0xFFFFFF),
    danger_fill: hex(0xC42B1C),
};

/// The palette of the appearance on screen.
pub fn palette(context: &egui::Context) -> &'static Palette {
    match context.theme() {
        Theme::Dark => &DARK,
        Theme::Light => &LIGHT,
    }
}

pub const CORNER: u8 = 6;
pub const SECTION_CORNER: u8 = 9;

const MEDIUM: &str = "huda-medium";
const SEMIBOLD: &str = "huda-semibold";

pub fn medium() -> FontFamily {
    FontFamily::Name(MEDIUM.into())
}
pub fn semibold() -> FontFamily {
    FontFamily::Name(SEMIBOLD.into())
}

/// A body-sized emphasis, the macOS Headline role.
pub fn headline() -> FontId {
    FontId::new(13.0, semibold())
}
/// A section title, the macOS Subheadline role with its emphasized weight.
pub fn section_title() -> FontId {
    FontId::new(11.0, semibold())
}
/// The supporting line of a row, the macOS Subheadline role.
pub fn subheadline() -> FontId {
    FontId::new(11.0, FontFamily::Proportional)
}

/// The system's own text faces, which a person already reads everywhere else. Each is read once
/// at launch; a system without them keeps the bundled faces.
struct Faces {
    regular: FontData,
    medium: FontData,
    semibold: FontData,
    mono: Option<FontData>,
}

fn weighted(bytes: &'static [u8], weight: f32) -> FontData {
    FontData::from_static(bytes).tweak(FontTweak {
        // SF defaults to its display cut; 17 is the text cut this app's sizes need.
        coords: VariationCoords::new([(b"wght", weight), (b"opsz", 17.0)]),
        ..Default::default()
    })
}

#[cfg(target_os = "macos")]
fn system_faces() -> Option<Faces> {
    let bytes: &'static [u8] = Vec::leak(std::fs::read("/System/Library/Fonts/SFNS.ttf").ok()?);
    let mono = std::fs::read("/System/Library/Fonts/SFNSMono.ttf")
        .ok()
        .map(FontData::from_owned);
    Some(Faces {
        regular: weighted(bytes, 400.0),
        medium: weighted(bytes, 510.0),
        semibold: weighted(bytes, 590.0),
        mono,
    })
}

#[cfg(windows)]
fn system_faces() -> Option<Faces> {
    let fonts = std::path::Path::new(&std::env::var_os("WINDIR")?).join("Fonts");
    let face = |name: &str| {
        std::fs::read(fonts.join(name))
            .ok()
            .map(FontData::from_owned)
    };
    Some(Faces {
        regular: face("segoeui.ttf")?,
        medium: face("segoeui.ttf")?,
        semibold: face("seguisb.ttf")?,
        mono: face("consola.ttf"),
    })
}

#[cfg(not(any(target_os = "macos", windows)))]
fn system_faces() -> Option<Faces> {
    None
}

/// The emphasized families always exist, because drawing with an unknown family panics.
fn definitions(faces: Option<Faces>) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let fallback = fonts.families[&FontFamily::Proportional].clone();
    let mut family = |name: &str, list: Vec<String>| {
        fonts.families.insert(FontFamily::Name(name.into()), list);
    };
    family(MEDIUM, fallback.clone());
    family(SEMIBOLD, fallback);
    if let Some(faces) = faces {
        for (name, face, target) in [
            ("system", faces.regular, FontFamily::Proportional),
            ("system-medium", faces.medium, medium()),
            ("system-semibold", faces.semibold, semibold()),
        ] {
            fonts.font_data.insert(name.to_owned(), Arc::new(face));
            if let Some(list) = fonts.families.get_mut(&target) {
                list.insert(0, name.to_owned());
            }
        }
        if let Some(mono) = faces.mono {
            fonts
                .font_data
                .insert("system-mono".to_owned(), Arc::new(mono));
            if let Some(list) = fonts.families.get_mut(&FontFamily::Monospace) {
                list.insert(0, "system-mono".to_owned());
            }
        }
    }
    fonts
}

fn visuals(base: Visuals, palette: &Palette) -> Visuals {
    let corner = CornerRadius::same(CORNER);
    let mut visuals = base;
    visuals.panel_fill = palette.window;
    visuals.window_fill = palette.surface;
    visuals.window_stroke = Stroke::new(1.0, palette.separator);
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.extreme_bg_color = palette.surface;
    visuals.faint_bg_color = palette.control_hover;
    visuals.code_bg_color = palette.control_hover;
    visuals.weak_text_color = Some(palette.tertiary);
    visuals.hyperlink_color = palette.accent_text;
    visuals.warn_fg_color = palette.warning;
    visuals.error_fg_color = palette.danger;
    visuals.selection.bg_fill = palette.selection;
    visuals.selection.stroke = Stroke::new(1.0, palette.accent_text);
    visuals.text_cursor.stroke = Stroke::new(1.5, palette.accent_text);
    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.separator);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.text);
    widgets.noninteractive.corner_radius = corner;
    for (state, fill) in [
        (&mut widgets.inactive, palette.control),
        (&mut widgets.hovered, palette.control_hover),
        (&mut widgets.active, palette.control_hover),
        (&mut widgets.open, palette.control_hover),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, palette.separator);
        state.fg_stroke = Stroke::new(1.0, palette.text);
        state.corner_radius = corner;
        state.expansion = 0.0;
    }
    visuals
}

fn style_for(context: &egui::Context, theme: Theme, palette: &Palette) {
    let mut style = (*context.style_of(theme)).clone();
    style.visuals = visuals(theme.default_visuals(), palette);
    // The macOS text styles: Title 1, Body, Subheadline and a callout-sized code face.
    style.text_styles = [
        (TextStyle::Heading, FontId::new(22.0, semibold())),
        (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(13.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(12.0, FontFamily::Monospace),
        ),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(12.0, 5.0);
    // The default macOS control height.
    style.spacing.interact_size.y = 28.0;
    style.animation_time = 0.1;
    context.set_style_of(theme, style);
}

/// Installs the fonts and both appearances. The system chooses between them.
pub fn apply(context: &egui::Context) {
    context.set_fonts(definitions(system_faces()));
    context.options_mut(|options| options.theme_preference = ThemePreference::System);
    style_for(context, Theme::Dark, &DARK);
    style_for(context, Theme::Light, &LIGHT);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(value: u8) -> f64 {
        let value = f64::from(value) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }

    fn luminance(color: Color32) -> f64 {
        0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    fn check(name: &str, palette: &Palette) {
        let backgrounds = [
            ("window", palette.window),
            ("sidebar", palette.sidebar),
            ("surface", palette.surface),
        ];
        let texts = [
            ("text", palette.text),
            ("secondary", palette.secondary),
            ("tertiary", palette.tertiary),
            ("accent_text", palette.accent_text),
            ("success", palette.success),
            ("warning", palette.warning),
            ("danger", palette.danger),
        ];
        for (background_name, background) in backgrounds {
            for (text_name, text) in texts {
                let ratio = contrast(text, background);
                assert!(
                    ratio >= 4.5,
                    "{name}: {text_name} on {background_name} is {ratio:.2}:1"
                );
            }
        }
        for (pair, text, background, least) in [
            ("text on control", palette.text, palette.control, 4.5),
            ("text on hover", palette.text, palette.control_hover, 4.5),
            (
                "secondary on control",
                palette.secondary,
                palette.control,
                4.5,
            ),
            ("text on selection", palette.text, palette.selection, 4.5),
            ("primary button", palette.on_accent, palette.accent, 4.5),
            (
                "destructive button",
                palette.on_danger,
                palette.danger_fill,
                4.5,
            ),
            // A shape that carries meaning needs 3:1 against what it sits on.
            (
                "crossbar on surface",
                palette.crossbar,
                palette.surface,
                3.0,
            ),
            (
                "crossbar on sidebar",
                palette.crossbar,
                palette.sidebar,
                3.0,
            ),
        ] {
            let ratio = contrast(text, background);
            assert!(ratio >= least, "{name}: {pair} is {ratio:.2}:1");
        }
    }

    #[test]
    fn every_text_pair_meets_its_contrast_in_both_appearances() {
        check("light", &LIGHT);
        check("dark", &DARK);
    }

    #[test]
    fn a_system_without_its_fonts_still_has_every_family() {
        let fonts = definitions(None);
        for family in [medium(), semibold()] {
            assert!(!fonts.families[&family].is_empty());
        }
    }

    #[test]
    fn both_appearances_install_without_a_window() {
        let context = egui::Context::default();
        apply(&context);
        for theme in [Theme::Light, Theme::Dark] {
            let style = context.style_of(theme);
            assert_eq!(style.spacing.interact_size.y, 28.0);
            assert_eq!(style.text_styles[&TextStyle::Body].size, 13.0);
        }
    }
}
