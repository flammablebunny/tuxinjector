// Action bindings and command dispatch for the Lua runtime.
// Key combos get mapped to callback IDs here, and commands flow
// back to the render thread through crossbeam channels.

// Commands produced by Lua callbacks, drained per-frame on the render thread
#[derive(Debug, Clone)]
pub enum TuxinjectorCommand {
    SwitchMode(String),
    ToggleMode { main: String, fallback: String },
    SetSensitivity(f32), // 0.0 resets to config default
    ToggleGui,
    Exec(String),  // fire & forget subprocess
    ToggleAppVisibility,
    PressKey(i32), // synthetic press+release
    Log(String),
}

// A single keybinding registered via tx.bind() in Lua
#[derive(Debug, Clone)]
pub struct LuaActionBinding {
    pub key_combo: Vec<i32>,     // GLFW keycodes -- all must be held simultaneously
    pub callback_id: u64,
    pub block_from_game: bool,   // swallow the key event so the game never sees it
}

// Collects tx.bind() calls while we're evaluating the config.
// Grab the result with into_bindings() when you're done.
#[derive(Default)]
pub struct ActionBuilder {
    bindings: Vec<LuaActionBinding>,
    next_id: u64,
}

impl ActionBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    // Returns callback ID so the caller can stash the corresponding Lua function
    pub fn register(&mut self, key_combo: Vec<i32>, block: bool) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.bindings.push(LuaActionBinding {
            key_combo,
            callback_id: id,
            block_from_game: block,
        });
        id
    }

    pub fn bindings(&self) -> &[LuaActionBinding] {
        &self.bindings
    }

}
