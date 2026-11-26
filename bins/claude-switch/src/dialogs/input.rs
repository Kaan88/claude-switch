use super::*;

pub fn text_input(st: &mut AppState, text: &str) {
    if let Some(Dialog::NewSession(mut picker)) = st.dialog.take_if(|d| matches!(d, Dialog::NewSession(_))) {
        picker.text_input(st, text);
        st.dialog = Some(Dialog::NewSession(picker));
        return;
    }
    if let Some(Dialog::Session(mut panel)) = st.dialog.take_if(|d| matches!(d, Dialog::Session(_))) {
        panel.text_input(st, text);
        st.dialog = Some(Dialog::Session(panel));
        return;
    }
    match &mut st.dialog {
        Some(Dialog::Name { field, .. }) => {
            field.insert(text);
        }
        Some(Dialog::Compact(compact)) => compact.text_input(text),
        _ => {}
    }
}

pub fn wheel(st: &mut AppState, x: f32, y: f32, dy: f32) {
    match &mut st.dialog {
        Some(Dialog::Settings(form)) => {
            form.wheel(x, y, -dy);
        }
        Some(Dialog::Accounts { scroll, .. }) => *scroll = (*scroll - dy).max(0.0),
        Some(Dialog::Session(panel)) => panel.wheel(dy),
        Some(Dialog::Compact(compact)) => compact.wheel(dy),
        Some(Dialog::Confirm { .. }) => st.confirm_scroll = (st.confirm_scroll - dy).max(0.0),
        _ => {}
    }
}
