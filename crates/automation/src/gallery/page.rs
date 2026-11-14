//! The gallery's `index.html`.

use std::fmt::Write as _;
use std::path::PathBuf;

use test_harness::OmarchyTheme;

use super::indicator::IndicatorState;
use super::{Look, State};

/// The gallery page: one row per state, stock light and dark, then Omarchy
/// light and dark; then the Indicator, one row per state, Omarchy light and
/// dark, enlarged without smoothing so each pixel shows.
pub(super) fn index_html(
    shots: &[(&State, Look, PathBuf)],
    indicators: &[(&IndicatorState, OmarchyTheme, PathBuf)],
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
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>Desktop Calls screenshot gallery</title>\n\
         <style>body{{font-family:sans-serif;margin:16px}}\
         main{{display:grid;grid-template-columns:repeat(4,1fr);gap:16px}}\
         img{{width:100%;border:1px solid #888}}\
         img.icon{{image-rendering:pixelated}}\
         main.indicator{{grid-template-columns:repeat(2,minmax(0,384px))}}</style>\n\
         </head>\n<body>\n\
         <h1>Desktop Calls screenshot gallery</h1>\n<main>\n{figures}</main>\n\
         <h2>The Indicator in Omarchy's bar</h2>\n\
         <p>Each icon as Omarchy's tray draws it: the SVG the app names, 12 logical pixels \
         at 2× scale, painted in the bar's foreground through its alpha mask.</p>\n\
         <main class=\"indicator\">\n{icons}</main>\n</body>\n</html>\n"
    )
}
