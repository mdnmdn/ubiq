//! The semantic lens control (M6, `V/semantic-lens.js`): `l`, or the pill at the picture's
//! top-left, steps through the filters the diagram offers (each node kind, then each authored
//! tag, most populated first) and back to off. The selection sets are the core's
//! (`ubiq_archify::motion::lens`); the dim, the matching comets and the 24-edge cap come from the
//! motion state; this file is the control and its words.

use gpui::{Context, ElementId, IntoElement};
use crate::app::AppState;
use crate::ui::kit::choice_pill;

use crate::state::archify::animate;

/// What the pill says: `Lens` when off, `Lens: backend` (or `Lens: #public`) when on.
pub fn pill_label(active: Option<&str>) -> String {
    match active {
        Some(filter) => format!("Lens: {filter}"),
        None => "Lens".to_string(),
    }
}

/// The pill. A click steps to the next filter, like the `l` key; it is lit while a lens is on.
pub fn pill(key: &str, active: Option<String>, cx: &mut Context<AppState>) -> impl IntoElement {
    let entity = cx.entity();
    let key = key.to_string();
    choice_pill(
        ElementId::Name("archify-lens".into()),
        pill_label(active.as_deref()),
        active.is_some(),
        move |_, _, cx| {
            entity.update(cx, |this, cx| animate::cycle_lens(this, &key, cx));
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pill_names_the_filter_that_is_on() {
        assert_eq!(pill_label(None), "Lens");
        assert_eq!(pill_label(Some("backend")), "Lens: backend");
        assert_eq!(pill_label(Some("#public")), "Lens: #public");
    }
}
