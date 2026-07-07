// Ninjabrain Bot API data model, snapshot publishing and the event parsers.
// Ported straight from toolscreen (ninjabrain_data.{h,cpp} + the parse fns in
// ninjabrain_api.cpp). Kept pure/no-I/O on purpose so it's all unit-testable:
// nbb_client.rs shoves JSON events in one side, the overlay reads lock-free
// snapshots out the other.

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde_json::Value;
use tuxinjector_core::rcu::RcuCell;

pub const PREDICTION_LIMIT: usize = 5;
pub const THROW_LIMIT: usize = 8;
pub const INFO_MSG_LIMIT: usize = 8;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NbbPrediction {
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub certainty: f64,
    pub overworld_distance: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NbbThrow {
    pub x_in_overworld: f64,
    pub z_in_overworld: f64,
    pub has_position: bool,
    pub angle: f64,
    pub angle_without_correction: f64,
    pub correction: f64,
    pub error: f64,
    pub correction_increments: i32,
    pub has_correction_increments: bool,
    pub throw_type: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NbbPredictionAngle {
    pub actual_angle: f64,
    pub needed_correction: f64,
    pub valid: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NbbInfoMessage {
    pub severity: String,
    pub msg_type: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NbbBlind {
    pub enabled: bool,
    pub has_divine: bool,
    pub has_result: bool,
    pub evaluation: String,
    pub x_in_nether: f64,
    pub z_in_nether: f64,
    pub improve_distance: f64,
    pub average_distance: f64,
    pub improve_direction: f64,
    pub highroll_probability: f64,
    pub highroll_threshold: f64,
}

#[derive(Clone, Debug)]
pub struct NinjabrainData {
    pub stronghold_x: i32,
    pub stronghold_z: i32,
    pub distance: f64,
    pub certainty: f64,

    pub predictions: Vec<NbbPrediction>,
    pub prediction_angles: Vec<NbbPredictionAngle>,
    pub prediction_count: i32,

    pub throws: Vec<NbbThrow>,
    pub eye_count: i32,

    pub last_angle: f64,
    pub prev_angle: f64,
    pub has_angle_change: bool,

    pub last_correction: f64,
    pub last_throw_error: f64,
    pub last_angle_without_correction: f64,
    pub has_correction: bool,
    pub has_throw_error: bool,

    pub has_nether_angle: bool,
    pub nether_angle: f64,
    pub nether_angle_diff: f64,

    pub player_x: f64,
    pub player_z: f64,
    pub player_horizontal_angle: f64,
    pub player_in_nether: bool,
    pub has_player_pos: bool,

    pub information_messages: Vec<NbbInfoMessage>,

    pub blind: NbbBlind,

    // NB 1.5.1 doesn't send us increment counts, so we recover them ourselves:
    // step ±1 each SSE event based on the sign of the last throw's correction
    // delta, and reset whenever eye_count changes. Ignored once the API starts
    // sending has_correction_increments.
    pub correction_increments_151: i32,

    pub result_type: String,
    pub valid_prediction: bool,

    pub boat_state: String,
    pub boat_angle: f64,
    pub has_boat_angle: bool,

    pub last_update: Option<Instant>,
}

impl Default for NinjabrainData {
    fn default() -> Self {
        Self {
            stronghold_x: 0,
            stronghold_z: 0,
            distance: 0.0,
            certainty: 0.0,
            predictions: Vec::new(),
            prediction_angles: Vec::new(),
            prediction_count: 0,
            throws: Vec::new(),
            eye_count: 0,
            last_angle: 0.0,
            prev_angle: 0.0,
            has_angle_change: false,
            last_correction: 0.0,
            last_throw_error: 0.0,
            last_angle_without_correction: 0.0,
            has_correction: false,
            has_throw_error: false,
            has_nether_angle: false,
            nether_angle: 0.0,
            nether_angle_diff: 0.0,
            player_x: 0.0,
            player_z: 0.0,
            player_horizontal_angle: 0.0,
            player_in_nether: false,
            has_player_pos: false,
            information_messages: Vec::new(),
            blind: NbbBlind::default(),
            correction_increments_151: 0,
            result_type: "NONE".to_string(),
            valid_prediction: false,
            boat_state: "NONE".to_string(),
            boat_angle: 0.0,
            has_boat_angle: false,
            last_update: None,
        }
    }
}

impl NinjabrainData {
    // Nether coords are 1/8 the overworld scale, so scale the distance down too.
    pub fn display_distance(&self, p: &NbbPrediction) -> f64 {
        if self.player_in_nether {
            p.overworld_distance / 8.0
        } else {
            p.overworld_distance
        }
    }
}

// Snapshot publishing - this is toolscreen's ModifyNinjabrainData.

fn cell() -> &'static RcuCell<NinjabrainData> {
    static C: OnceLock<RcuCell<NinjabrainData>> = OnceLock::new();
    C.get_or_init(|| RcuCell::new(NinjabrainData::default()))
}

fn write_lock() -> &'static Mutex<()> {
    static M: Mutex<()> = Mutex::new(());
    &M
}

/// Read-modify-write under a mutex so the four SSE streams can share one
/// struct without stomping each other. Render thread reads lock-free.
pub fn modify_nbb_data(f: impl FnOnce(&mut NinjabrainData)) {
    let _g = write_lock().lock().unwrap();
    let mut next: NinjabrainData = (**cell().load()).clone();
    f(&mut next);
    next.last_update = Some(Instant::now());
    cell().publish(next);
}

/// Publish a fresh empty snapshot (client start).
pub fn reset_nbb_data() {
    let _g = write_lock().lock().unwrap();
    cell().publish(NinjabrainData::default());
}

pub fn nbb_snapshot() -> std::sync::Arc<NinjabrainData> {
    std::sync::Arc::clone(&cell().load())
}

// --- lenient JSON helpers (nlohmann json.value() semantics) ---

fn jv_f64(v: &Value, key: &str, default: f64) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(default)
}
fn jv_i32(v: &Value, key: &str, default: i32) -> i32 {
    v.get(key).and_then(Value::as_i64).map(|n| n as i32).unwrap_or(default)
}
fn jv_bool(v: &Value, key: &str, default: bool) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(default)
}
fn jv_str(v: &Value, key: &str, default: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| default.to_string())
}

/// Wrap an angle to [-180, 180].
pub fn normalize_angle_degrees(mut a: f64) -> f64 {
    while a > 180.0 {
        a -= 360.0;
    }
    while a < -180.0 {
        a += 360.0;
    }
    a
}

// --- per-stream clears (fired on that stream's disconnect) ---

/// Reset stronghold-derived fields, preserving boat / blind / info messages.
pub fn clear_stronghold(d: &mut NinjabrainData) {
    let boat_state = std::mem::take(&mut d.boat_state);
    let boat_angle = d.boat_angle;
    let has_boat_angle = d.has_boat_angle;
    let info = std::mem::take(&mut d.information_messages);
    let blind = std::mem::take(&mut d.blind);
    *d = NinjabrainData::default();
    d.boat_state = boat_state;
    d.boat_angle = boat_angle;
    d.has_boat_angle = has_boat_angle;
    d.information_messages = info;
    d.blind = blind;
}

pub fn clear_boat(d: &mut NinjabrainData) {
    d.boat_state = "NONE".to_string();
    d.boat_angle = 0.0;
    d.has_boat_angle = false;
}

pub fn clear_info_messages(d: &mut NinjabrainData) {
    d.information_messages.clear();
}

pub fn clear_blind(d: &mut NinjabrainData) {
    d.blind = NbbBlind::default();
}

// --- event parsers (ports of ninjabrain_api.cpp) ---

/// Stronghold event. `d` is the current shared struct; toolscreen builds a
/// fresh `next` and carries boat/blind/info over — we do the same in place.
pub fn apply_stronghold_event(d: &mut NinjabrainData, json: &Value) {
    let result_type = jv_str(json, "resultType", "NONE");
    if result_type == "NONE" {
        clear_stronghold(d);
        d.result_type = result_type;
        return;
    }

    let previous_eye_count = d.eye_count;
    let previous_increments_151 = d.correction_increments_151;
    let previous_last_correction = if d.eye_count > 0 {
        d.throws.get(d.eye_count as usize - 1).map(|t| t.correction)
    } else {
        None
    };

    // fresh stronghold section
    let mut next = NinjabrainData {
        boat_state: std::mem::take(&mut d.boat_state),
        boat_angle: d.boat_angle,
        has_boat_angle: d.has_boat_angle,
        information_messages: std::mem::take(&mut d.information_messages),
        blind: std::mem::take(&mut d.blind),
        ..NinjabrainData::default()
    };
    next.result_type = result_type;

    if let Some(pp) = json.get("playerPosition").filter(|p| p.is_object() && !p.as_object().unwrap().is_empty()) {
        next.player_x = jv_f64(pp, "xInOverworld", 0.0);
        next.player_z = jv_f64(pp, "zInOverworld", 0.0);
        next.player_in_nether = jv_bool(pp, "isInNether", false);
        next.player_horizontal_angle = jv_f64(pp, "horizontalAngle", 0.0);
        next.has_player_pos = true;
    }

    if let Some(throws) = json.get("eyeThrows").and_then(Value::as_array) {
        next.eye_count = throws.len().min(THROW_LIMIT) as i32;
        for tj in throws.iter().take(THROW_LIMIT) {
            let mut t = NbbThrow {
                x_in_overworld: jv_f64(tj, "xInOverworld", next.player_x),
                z_in_overworld: jv_f64(tj, "zInOverworld", next.player_z),
                has_position: tj.get("xInOverworld").is_some()
                    || tj.get("zInOverworld").is_some()
                    || next.has_player_pos,
                angle: jv_f64(tj, "angle", 0.0),
                correction: jv_f64(tj, "correction", 0.0),
                throw_type: jv_str(tj, "type", "NORMAL"),
                ..NbbThrow::default()
            };
            t.angle_without_correction = jv_f64(tj, "angleWithoutCorrection", t.angle);
            t.error = jv_f64(tj, "error", t.correction);
            if let Some(ci) = tj.get("correctionIncrements").filter(|v| !v.is_null()) {
                t.correction_increments = ci.as_i64().unwrap_or(0) as i32;
                t.has_correction_increments = true;
            }
            next.throws.push(t);
        }
    }

    // NB 1.5.1 increment recovery using the previous snapshot
    if next.eye_count > 0 && !next.throws[next.eye_count as usize - 1].has_correction_increments {
        next.correction_increments_151 = previous_increments_151;
        if next.eye_count != previous_eye_count {
            next.correction_increments_151 = 0;
        } else if let Some(prev_corr) = previous_last_correction {
            let delta = next.throws[next.eye_count as usize - 1].correction - prev_corr;
            if delta > 1e-9 {
                next.correction_increments_151 += 1;
            } else if delta < -1e-9 {
                next.correction_increments_151 -= 1;
            }
        }
    }

    if next.eye_count >= 1 {
        let last = &next.throws[next.eye_count as usize - 1];
        next.last_angle = last.angle;
        next.last_angle_without_correction = last.angle_without_correction;
        next.last_correction = last.correction;
        next.last_throw_error = last.error;
        next.has_correction = next.last_correction.abs() > 1e-9;
        next.has_throw_error = next.last_throw_error.abs() > 1e-9;
        if next.eye_count >= 2 {
            next.prev_angle = next.throws[next.eye_count as usize - 2].angle;
            next.has_angle_change = true;
            next.has_nether_angle = true;
            next.nether_angle = next.last_angle;
            next.nether_angle_diff = next.last_angle - next.throws[0].angle;
        }
    }

    if let Some(preds) = json.get("predictions").and_then(Value::as_array) {
        next.prediction_count = preds.len().min(PREDICTION_LIMIT) as i32;
        for pj in preds.iter().take(PREDICTION_LIMIT) {
            let p = NbbPrediction {
                chunk_x: jv_i32(pj, "chunkX", 0),
                chunk_z: jv_i32(pj, "chunkZ", 0),
                certainty: jv_f64(pj, "certainty", 0.0),
                overworld_distance: jv_f64(pj, "overworldDistance", 0.0),
            };
            let mut ang = NbbPredictionAngle::default();
            if next.has_player_pos {
                let block_x = p.chunk_x as f64 * 16.0 + 4.0;
                let block_z = p.chunk_z as f64 * 16.0 + 4.0;
                let x_diff = block_x - next.player_x;
                let z_diff = block_z - next.player_z;
                let structure_angle = -x_diff.atan2(z_diff) * 180.0 / std::f64::consts::PI;
                ang.actual_angle = structure_angle;
                ang.needed_correction =
                    normalize_angle_degrees(structure_angle - next.player_horizontal_angle);
                ang.valid = true;
            }
            next.predictions.push(p);
            next.prediction_angles.push(ang);
        }
        if next.prediction_count > 0 {
            next.stronghold_x = next.predictions[0].chunk_x * 16 + 4;
            next.stronghold_z = next.predictions[0].chunk_z * 16 + 4;
            next.distance = next.predictions[0].overworld_distance;
            next.certainty = next.predictions[0].certainty;
            next.valid_prediction = true;
        }
    }

    *d = next;
}

pub fn apply_boat_event(d: &mut NinjabrainData, json: &Value) {
    d.boat_state = jv_str(json, "boatState", "NONE");
    match json.get("boatAngle").filter(|v| !v.is_null()).and_then(Value::as_f64) {
        Some(a) => {
            d.boat_angle = a;
            d.has_boat_angle = true;
        }
        None => {
            d.boat_angle = 0.0;
            d.has_boat_angle = false;
        }
    }
}

pub fn apply_info_messages_event(d: &mut NinjabrainData, json: &Value) {
    d.information_messages.clear();
    if let Some(msgs) = json.get("informationMessages").and_then(Value::as_array) {
        for mj in msgs.iter().take(INFO_MSG_LIMIT) {
            d.information_messages.push(NbbInfoMessage {
                severity: jv_str(mj, "severity", "INFO"),
                msg_type: jv_str(mj, "type", ""),
                message: jv_str(mj, "message", ""),
            });
        }
    }
}

pub fn apply_blind_event(d: &mut NinjabrainData, json: &Value) {
    let mut blind = NbbBlind {
        enabled: jv_bool(json, "isBlindModeEnabled", false),
        has_divine: jv_bool(json, "hasDivine", false),
        ..NbbBlind::default()
    };
    if let Some(br) = json.get("blindResult").filter(|p| p.is_object() && !p.as_object().unwrap().is_empty()) {
        blind.has_result = true;
        blind.evaluation = jv_str(br, "evaluation", "");
        blind.x_in_nether = jv_f64(br, "xInNether", 0.0);
        blind.z_in_nether = jv_f64(br, "zInNether", 0.0);
        blind.improve_distance = jv_f64(br, "improveDistance", 0.0);
        blind.average_distance = jv_f64(br, "averageDistance", 0.0);
        blind.improve_direction = jv_f64(br, "improveDirection", 0.0);
        blind.highroll_probability = jv_f64(br, "highrollProbability", 0.0);
        blind.highroll_threshold = jv_f64(br, "highrollThreshold", 0.0);
    }
    if !blind.enabled {
        blind = NbbBlind::default();
    }
    d.blind = blind;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn none_result_clears_but_preserves_other_streams() {
        let mut d = NinjabrainData::default();
        d.boat_state = "VALID".into();
        d.blind.enabled = true;
        d.information_messages.push(NbbInfoMessage::default());
        d.valid_prediction = true;
        d.eye_count = 2;
        apply_stronghold_event(&mut d, &json!({"resultType": "NONE"}));
        assert!(!d.valid_prediction);
        assert_eq!(d.eye_count, 0);
        assert_eq!(d.result_type, "NONE");
        assert_eq!(d.boat_state, "VALID");
        assert!(d.blind.enabled);
        assert_eq!(d.information_messages.len(), 1);
    }

    #[test]
    fn triangulation_parse_full() {
        let mut d = NinjabrainData::default();
        d.boat_state = "MEASURING".into();
        let ev = json!({
            "resultType": "TRIANGULATION",
            "playerPosition": {"xInOverworld": 100.0, "zInOverworld": -50.0, "isInNether": false, "horizontalAngle": -45.0},
            "eyeThrows": [
                {"xInOverworld": 100.0, "zInOverworld": -50.0, "angle": -42.5, "angleWithoutCorrection": -42.0, "correction": -0.5, "error": 0.01, "type": "NORMAL"},
                {"angle": -40.0, "correction": 0.0}
            ],
            "predictions": [
                {"chunkX": 10, "chunkZ": -20, "certainty": 0.95, "overworldDistance": 400.0},
                {"chunkX": 11, "chunkZ": -21, "certainty": 0.05, "overworldDistance": 420.0}
            ]
        });
        apply_stronghold_event(&mut d, &ev);
        assert_eq!(d.result_type, "TRIANGULATION");
        assert!(d.valid_prediction);
        assert_eq!(d.eye_count, 2);
        assert_eq!(d.prediction_count, 2);
        assert_eq!(d.stronghold_x, 10 * 16 + 4);
        assert_eq!(d.stronghold_z, -20 * 16 + 4);
        assert!((d.certainty - 0.95).abs() < 1e-12);
        // derived
        assert!((d.last_angle - -40.0).abs() < 1e-12);
        assert!((d.prev_angle - -42.5).abs() < 1e-12);
        assert!(d.has_angle_change && d.has_nether_angle);
        assert!((d.nether_angle_diff - 2.5).abs() < 1e-12);
        // second throw defaults: x/z from player, error defaults to correction
        assert!((d.throws[1].x_in_overworld - 100.0).abs() < 1e-12);
        assert!(d.throws[1].has_position, "hasPlayerPos makes it positioned");
        assert!((d.throws[1].angle_without_correction - -40.0).abs() < 1e-12);
        // prediction angle math
        let ang = &d.prediction_angles[0];
        assert!(ang.valid);
        let block_x = 164.0;
        let block_z = -316.0;
        let expect = -((block_x - 100.0_f64).atan2(block_z - -50.0)) * 180.0 / std::f64::consts::PI;
        assert!((ang.actual_angle - expect).abs() < 1e-9);
        assert!(
            (ang.needed_correction - normalize_angle_degrees(expect - -45.0)).abs() < 1e-9
        );
        // boat preserved
        assert_eq!(d.boat_state, "MEASURING");
    }

    #[test]
    fn increments_151_recovery() {
        let mut d = NinjabrainData::default();
        let ev = |corr: f64| {
            json!({
                "resultType": "TRIANGULATION",
                "eyeThrows": [{"angle": 10.0, "correction": corr}],
                "predictions": []
            })
        };
        apply_stronghold_event(&mut d, &ev(0.0));
        assert_eq!(d.correction_increments_151, 0);
        // same eye count, correction up => +1
        apply_stronghold_event(&mut d, &ev(0.01));
        assert_eq!(d.correction_increments_151, 1);
        apply_stronghold_event(&mut d, &ev(0.02));
        assert_eq!(d.correction_increments_151, 2);
        // down => -1
        apply_stronghold_event(&mut d, &ev(0.01));
        assert_eq!(d.correction_increments_151, 1);
        // eye count change resets
        let two = json!({
            "resultType": "TRIANGULATION",
            "eyeThrows": [{"angle": 10.0, "correction": 0.01}, {"angle": 11.0, "correction": 0.0}],
            "predictions": []
        });
        apply_stronghold_event(&mut d, &two);
        assert_eq!(d.correction_increments_151, 0);
    }

    #[test]
    fn explicit_increments_skip_recovery() {
        let mut d = NinjabrainData::default();
        let ev = json!({
            "resultType": "TRIANGULATION",
            "eyeThrows": [{"angle": 10.0, "correction": 0.02, "correctionIncrements": 3}],
            "predictions": []
        });
        apply_stronghold_event(&mut d, &ev);
        assert!(d.throws[0].has_correction_increments);
        assert_eq!(d.throws[0].correction_increments, 3);
        assert_eq!(d.correction_increments_151, 0);
    }

    #[test]
    fn boat_null_angle() {
        let mut d = NinjabrainData::default();
        apply_boat_event(&mut d, &json!({"boatState": "ERROR", "boatAngle": null}));
        assert_eq!(d.boat_state, "ERROR");
        assert!(!d.has_boat_angle);
        apply_boat_event(&mut d, &json!({"boatState": "VALID", "boatAngle": 12.5}));
        assert!(d.has_boat_angle && (d.boat_angle - 12.5).abs() < 1e-12);
    }

    #[test]
    fn blind_disabled_clears() {
        let mut d = NinjabrainData::default();
        let ev = json!({
            "isBlindModeEnabled": true, "hasDivine": false,
            "blindResult": {"evaluation": "HIGHROLL_GOOD", "xInNether": 12.0, "zInNether": -34.0,
                "improveDistance": 40.0, "averageDistance": 200.0, "improveDirection": 90.0,
                "highrollProbability": 0.22, "highrollThreshold": 400.0}
        });
        apply_blind_event(&mut d, &ev);
        assert!(d.blind.enabled && d.blind.has_result);
        assert_eq!(d.blind.evaluation, "HIGHROLL_GOOD");
        apply_blind_event(&mut d, &json!({"isBlindModeEnabled": false}));
        assert_eq!(d.blind, NbbBlind::default());
    }

    #[test]
    fn info_messages_capped_at_8() {
        let mut d = NinjabrainData::default();
        let msgs: Vec<Value> = (0..12)
            .map(|i| json!({"severity": "INFO", "type": "X", "message": format!("m{i}")}))
            .collect();
        apply_info_messages_event(&mut d, &json!({"informationMessages": msgs}));
        assert_eq!(d.information_messages.len(), 8);
    }

    #[test]
    fn throws_capped_at_8() {
        let mut d = NinjabrainData::default();
        let throws: Vec<Value> = (0..10).map(|i| json!({"angle": i as f64})).collect();
        apply_stronghold_event(
            &mut d,
            &json!({"resultType": "TRIANGULATION", "eyeThrows": throws, "predictions": []}),
        );
        assert_eq!(d.eye_count, 8);
    }

    #[test]
    fn normalize_wraps() {
        assert!((normalize_angle_degrees(190.0) - -170.0).abs() < 1e-12);
        assert!((normalize_angle_degrees(-190.0) - 170.0).abs() < 1e-12);
        assert!((normalize_angle_degrees(360.0) - 0.0).abs() < 1e-12);
    }

    #[test]
    fn snapshot_roundtrip() {
        reset_nbb_data();
        modify_nbb_data(|d| d.eye_count = 3);
        let s = nbb_snapshot();
        assert_eq!(s.eye_count, 3);
        assert!(s.last_update.is_some());
        reset_nbb_data();
        assert_eq!(nbb_snapshot().eye_count, 0);
    }
}
