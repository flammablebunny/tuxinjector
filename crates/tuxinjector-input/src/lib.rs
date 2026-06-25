//! Input system: GLFW callback interception, hotkeys, rebinding, sensitivity

pub mod callbacks;
pub mod glfw_types;
pub mod hotkey;
pub mod rebind;
pub mod sensitivity;

pub use callbacks::{
    InputHandler, mouse_position, raw_mouse_position, register_input_handler, unregister_input_handler,
    update_key_rebinds, physical_key_for,
    set_gui_visible, gui_is_visible,
    set_gui_wants_keyboard, gui_wants_keyboard,
    set_popup_capturing_mouse, popup_capturing_mouse,
    push_gui_button_press, push_gui_button_release,
    take_gui_button_press, take_gui_button_release,
    push_gui_rbutton_press, push_gui_rbutton_release,
    take_gui_rbutton_press, take_gui_rbutton_release,
    push_gui_button_mods, take_gui_button_mods,
    push_gui_scroll, take_gui_scroll,
    push_gui_key, take_gui_keys, push_gui_char, take_gui_text,
    set_gui_capture_mode, is_gui_capture_mode, push_captured_key, take_captured_key,
    is_cursor_captured,
    force_cursor_visible, restore_game_cursor,
    set_mode_sensitivity, clear_mode_sensitivity,
    is_key_pressed, press_key_to_game,
    set_key_repeat_table, tick_key_repeat,
    warp_cursor,
};
pub use hotkey::{HotkeyAction, HotkeyEngine};
pub use rebind::KeyRebinder;
pub use sensitivity::SensitivityState;
