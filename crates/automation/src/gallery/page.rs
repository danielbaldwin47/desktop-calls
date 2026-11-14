//! The gallery's `index.html`.

use std::fmt::Write as _;
use std::path::PathBuf;

use test_harness::OmarchyTheme;

use super::indicator::IndicatorState;
use super::{Look, State};

/// The gallery page: one row per state, stock light and dark, then Omarchy
/// light and dark; then the Indicator, one row per state, Omarchy light and
/// dark, enlarged without smoothing so each pixel shows; then its menu,
/// one row per state, the stand-in for Omarchy's.
pub(super) fn index_html(
    shots: &[(&State, Look, PathBuf)],
    indicators: &[(&IndicatorState, OmarchyTheme, PathBuf)],
    menus: &[(&IndicatorState, OmarchyTheme, PathBuf)],
) -> String {
    let mut figures = String::new();
    for (state, look, path) in shots {
        let file = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let _ = writeln!(
            figures,
            "<figure><a href=\"{file}\"><img src=\"{file}\" alt=\"{title}, {look}\"></a>\
             <figcaption>{title} ({look})</figcaption></figure>",
            title = state.title,
            look = look.as_str(),
        );
    }
    let mut icons = String::new();
    for (state, theme, path) in indicators {
        let file = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let _ = writeln!(
            icons,
            "<figure><a href=\"{file}\"><img class=\"icon\" src=\"{file}\" \
             alt=\"{title}, {look}\"></a><figcaption>{title} ({look})</figcaption></figure>",
            title = state.title,
            look = theme.as_str(),
        );
    }
    let mut menu_figures = String::new();
    for (state, theme, path) in menus {
        let file = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let _ = writeln!(
            menu_figures,
            "<figure><a href=\"{file}\"><img class=\"menu\" src=\"{file}\" \
             alt=\"{title}, {look}, a stand-in\"></a>\
             <figcaption>{title} ({look}). Stand-in: drawn from the menu layout the app \
             publishes, in Omarchy's menu style; Omarchy's shell draws the real one.\
             </figcaption></figure>",
            title = state.title,
            look = theme.as_str(),
        );
    }
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>Desktop Calls screenshot gallery</title>\n\
         <style>body{{font-family:sans-serif;margin:16px}}\
         main{{display:grid;grid-template-columns:repeat(4,1fr);gap:16px}}\
         img{{width:100%;border:1px solid #888}}\
         img.icon{{image-rendering:pixelated}}\
         img.menu{{width:248px;border:0}}\
         main.indicator{{grid-template-columns:repeat(2,minmax(0,384px))}}</style>\n\
         </head>\n<body>\n\
         <h1>Desktop Calls screenshot gallery</h1>\n<main>\n{figures}</main>\n\
         <h2>The Indicator in Omarchy's bar</h2>\n\
         <p>Each icon as Omarchy's tray draws it: the SVG the app names, 12 logical pixels \
         at 2× scale, painted in the bar's foreground through its alpha mask.</p>\n\
         <main class=\"indicator\">\n{icons}</main>\n\
         <h2>The Indicator's menu: a stand-in</h2>\n\
         <p>Omarchy's shell draws the Indicator's menu, and the gallery runs none, so these \
         are stand-ins: the menu layout the app publishes over dbusmenu, read back as a tray \
         reads it, drawn in the style of Omarchy's tray menu (<code>Tray.qml</code>) at 2× \
         scale. A disabled entry is dimmed, <code>›</code> marks a submenu, and an opened \
         submenu replaces the menu under a <code>‹</code> header, as Omarchy draws them.</p>\n\
         <main class=\"indicator\">\n{menu_figures}</main>\n</body>\n</html>\n"
    )
}
