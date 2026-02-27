use whisper_rs::{WhisperContext, WhisperContextParameters};

fn main() {
    let ctx = WhisperContext::new_with_params("foo", WhisperContextParameters::default()).unwrap();
    let state = ctx.create_state().unwrap();
    let id = state.full_lang_id(); // Let's check if this exists
}
