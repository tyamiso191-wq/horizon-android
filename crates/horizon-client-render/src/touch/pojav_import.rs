//! Imports Pojav-style custom control layouts (JSON) as Horizon custom buttons.
//!
//! Written from the public description of the file format only; no code from
//! any other project was read or used.
use super::CustomButton;
use super::layout::{Control, Placement, Settings};
use serde_json::Value;

/// Screen size (dp) the layout expressions are evaluated against. Horizon
/// stores normalized positions, so the result scales to the real screen.
pub const REFERENCE_DP: [f32; 2] = [800.0, 360.0];
/// Gap between buttons in dp. Not verified against the original app.
pub const DEFAULT_MARGIN_DP: f32 = 2.0;
const MAX_BUTTONS: usize = 32;

#[derive(Debug, Default)]
pub struct Imported {
    pub buttons: Vec<CustomButton>,
    pub movement: Option<Placement>,
    pub skipped: Vec<String>,
}

impl Imported {
    pub fn apply(&self, s: &mut Settings) {
        s.custom_buttons = self.buttons.clone();
        if let Some(p) = self.movement {
            for row in &mut s.layouts {
                row[Control::Move as usize] = p;
            }
        }
    }
}

pub fn import_layout(json: &str, screen_dp: [f32; 2], margin_dp: f32) -> Result<Imported, String> {
    let root: Value = serde_json::from_str(json).map_err(|e| format!("not valid JSON: {e}"))?;
    let list = root
        .get("mControlDataList")
        .and_then(Value::as_array)
        .ok_or("missing mControlDataList")?;
    let mut out = Imported::default();
    for item in list {
        let name = item.get("name").and_then(Value::as_str).unwrap_or("?");
        match convert_button(item, screen_dp, margin_dp) {
            Ok(b) if out.buttons.len() < MAX_BUTTONS => out.buttons.push(b),
            Ok(_) => out.skipped.push(format!("{name}: over the {MAX_BUTTONS} button limit")),
            Err(why) => out.skipped.push(format!("{name}: {why}")),
        }
    }
    if let Some(drawers) = root.get("mDrawerDataList").and_then(Value::as_array)
        && !drawers.is_empty()
    {
        out.skipped.push(format!("{} drawer(s): not supported yet", drawers.len()));
    }
    if let Some(joy) = root
        .get("mJoystickDataList")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
    {
        match place(joy, screen_dp, margin_dp) {
            Ok(p) => out.movement = Some(p),
            Err(why) => out.skipped.push(format!("joystick: {why}")),
        }
    }
    Ok(out)
}

fn convert_button(item: &Value, screen: [f32; 2], margin: f32) -> Result<CustomButton, String> {
    let code = item
        .get("keycodes")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_i64)
        .ok_or("missing keycodes")?;
    let token = glfw_token(code).ok_or_else(|| format!("key code {code} has no Horizon equivalent"))?;
    if !CustomButton::valid_token(&token) {
        return Err(format!("{token} is not a Horizon key"));
    }
    let p = place(item, screen, margin)?;
    let mut b = CustomButton::new(token, 0);
    b.layouts = [p; 3];
    Ok(b)
}

fn place(item: &Value, screen: [f32; 2], margin: f32) -> Result<Placement, String> {
    let num = |k: &str, d: f32| item.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32);
    let (w, h) = (num("width", 50.0).max(1.0), num("height", 50.0).max(1.0));
    let vars = |name: &str| -> Option<f32> {
        match name {
            "margin" => Some(margin),
            "width" => Some(w),
            "height" => Some(h),
            "right" => Some(screen[0] - w),
            "bottom" => Some(screen[1] - h),
            _ => None,
        }
    };
    let expr = |key: &str| -> Result<f32, String> {
        let text = item
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("missing {key}"))?;
        eval(text, &vars)
    };
    let (x, y) = (expr("dynamicX")?, expr("dynamicY")?);
    let size = (w * h).sqrt() / screen[1];
    let mut p = Placement::new((x + w / 2.0) / screen[0], (y + h / 2.0) / screen[1], size);
    p.opacity = num("opacity", 1.0);
    p.sanitize();
    Ok(p)
}

/// GLFW key codes are a public, fixed list; Horizon uses Minecraft key names.
fn glfw_token(code: i64) -> Option<String> {
    let name = match code {
        48..=57 | 65..=90 => {
            let c = code as u8 as char;
            return Some(format!("key.keyboard.{}", c.to_ascii_lowercase()));
        }
        290..=301 => return Some(format!("key.keyboard.f{}", code - 289)),
        32 => "space",
        39 => "apostrophe",
        44 => "comma",
        45 => "minus",
        46 => "period",
        47 => "slash",
        59 => "semicolon",
        61 => "equal",
        91 => "left.bracket",
        92 => "backslash",
        93 => "right.bracket",
        96 => "grave.accent",
        256 => "escape",
        257 => "enter",
        258 => "tab",
        259 => "backspace",
        260 => "insert",
        261 => "delete",
        262 => "right",
        263 => "left",
        264 => "down",
        265 => "up",
        266 => "page.up",
        267 => "page.down",
        268 => "home",
        269 => "end",
        280 => "caps.lock",
        340 => "left.shift",
        341 => "left.control",
        342 => "left.alt",
        344 => "right.shift",
        345 => "right.control",
        346 => "right.alt",
        _ => return None,
    };
    Some(format!("key.keyboard.{name}"))
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    vars: &'a dyn Fn(&str) -> Option<f32>,
}

impl Parser<'_> {
    fn peek(&mut self) -> Option<u8> {
        while self.s.get(self.i).is_some_and(u8::is_ascii_whitespace) {
            self.i += 1;
        }
        self.s.get(self.i).copied()
    }
    fn expr(&mut self) -> Result<f32, String> {
        let mut v = self.term()?;
        while let Some(op @ (b'+' | b'-')) = self.peek() {
            self.i += 1;
            let r = self.term()?;
            if op == b'+' {
                v += r;
            } else {
                v -= r;
            }
        }
        Ok(v)
    }
    fn term(&mut self) -> Result<f32, String> {
        let mut v = self.factor()?;
        while let Some(op @ (b'*' | b'/')) = self.peek() {
            self.i += 1;
            let r = self.factor()?;
            if op == b'*' {
                v *= r;
            } else {
                v /= r;
            }
        }
        Ok(v)
    }
    fn factor(&mut self) -> Result<f32, String> {
        match self.peek() {
            Some(b'-') => {
                self.i += 1;
                Ok(-self.factor()?)
            }
            Some(b'(') => {
                self.i += 1;
                let v = self.expr()?;
                if self.peek() != Some(b')') {
                    return Err("missing )".into());
                }
                self.i += 1;
                Ok(v)
            }
            Some(b'$') => {
                if self.s.get(self.i + 1) != Some(&b'{') {
                    return Err("bad variable".into());
                }
                let start = self.i + 2;
                let end = self.s[start..]
                    .iter()
                    .position(|&c| c == b'}')
                    .ok_or("unterminated variable")?
                    + start;
                let name = std::str::from_utf8(&self.s[start..end]).map_err(|e| e.to_string())?;
                self.i = end + 1;
                (self.vars)(name).ok_or_else(|| format!("unknown variable ${{{name}}}"))
            }
            Some(c) if c.is_ascii_digit() || c == b'.' => {
                let start = self.i;
                while self
                    .s
                    .get(self.i)
                    .is_some_and(|c| c.is_ascii_digit() || *c == b'.')
                {
                    self.i += 1;
                }
                std::str::from_utf8(&self.s[start..self.i])
                    .unwrap_or("")
                    .parse()
                    .map_err(|_| "bad number".to_string())
            }
            _ => Err("unexpected character".into()),
        }
    }
}

fn eval(text: &str, vars: &dyn Fn(&str) -> Option<f32>) -> Result<f32, String> {
    let mut p = Parser { s: text.as_bytes(), i: 0, vars };
    let v = p.expr()?;
    if p.peek().is_some() {
        return Err(format!("unexpected text in {text:?}"));
    }
    if v.is_finite() {
        Ok(v)
    } else {
        Err(format!("{text:?} is not a finite number"))
    }
}

impl Settings {
    /// Imports `pojav-controls.json` placed next to the settings file, once.
    /// The result is saved and the source renamed to `*.imported`.
    pub(super) fn import_pojav_layout(&mut self, settings_path: &std::path::Path) {
        let file = settings_path.with_file_name("pojav-controls.json");
        let Ok(text) = std::fs::read_to_string(&file) else {
            return;
        };
        let report = settings_path.with_file_name("pojav-import-report.txt");
        match import_layout(&text, REFERENCE_DP, DEFAULT_MARGIN_DP) {
            Ok(imported) => {
                imported.apply(self);
                self.sanitize();
                let saved = serde_json::to_vec_pretty(&*self)
                    .ok()
                    .is_some_and(|bytes| std::fs::write(settings_path, bytes).is_ok());
                let mut lines = vec![format!(
                    "imported {} button(s), movement control: {}, saved: {saved}",
                    imported.buttons.len(),
                    imported.movement.is_some()
                )];
                lines.extend(imported.skipped.iter().map(|s| format!("skipped {s}")));
                let _ = std::fs::write(&report, lines.join("\n"));
                if saved {
                    let _ = std::fs::rename(&file, file.with_extension("json.imported"));
                }
            }
            Err(e) => {
                let _ = std::fs::write(&report, format!("import failed: {e}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"version":8,"scaledAt":100.0,"mControlDataList":[
      {"name":"W","keycodes":[87,0,0,0],"dynamicX":"${margin} * 2 + ${width}","dynamicY":"${bottom} - ${margin} * 3 - ${height} * 2","isToggle":true,"opacity":1.0,"width":50.0,"height":50.0},
      {"name":"A","keycodes":[65,0,0,0],"dynamicX":"${margin}","dynamicY":"${bottom} - ${margin} * 2 - ${height}","opacity":1.0,"width":50.0,"height":50.0},
      {"name":"Menu","keycodes":[-9,0,0,0],"dynamicX":"${right}","dynamicY":"${margin}","width":50.0,"height":50.0}],
      "mDrawerDataList":[],"mJoystickDataList":[]}"#;

    fn vars(name: &str) -> Option<f32> {
        match name {
            "margin" => Some(2.0),
            "width" => Some(50.0),
            _ => None,
        }
    }

    #[test]
    fn expressions_follow_precedence_and_reject_garbage() {
        assert_eq!(eval("${margin} * 2 + ${width}", &vars), Ok(54.0));
        assert_eq!(eval("2 + 3 * 4", &vars), Ok(14.0));
        assert_eq!(eval("(2 + 3) * -4", &vars), Ok(-20.0));
        assert!(eval("${nope}", &vars).is_err());
        assert!(eval("1 / 0", &vars).is_err());
        assert!(eval("1 +", &vars).is_err());
        assert!(eval("1 2", &vars).is_err());
    }

    #[test]
    fn imports_keys_and_reports_what_it_cannot_map() {
        let imported = import_layout(SAMPLE, REFERENCE_DP, DEFAULT_MARGIN_DP).unwrap();
        assert_eq!(imported.buttons.len(), 2);
        assert_eq!(imported.buttons[0].key, "key.keyboard.w");
        assert_eq!(imported.buttons[1].key, "key.keyboard.a");
        assert_eq!(imported.skipped.len(), 1);
        let w = imported.buttons[0].layouts[0];
        assert!((w.center[0] - 79.0 / 800.0).abs() < 1e-4);
        assert!((w.center[1] - 229.0 / 360.0).abs() < 1e-4);
        assert!((w.size - 50.0 / 360.0).abs() < 1e-4);
    }

    #[test]
    fn broken_input_is_an_error_not_a_panic() {
        assert!(import_layout("not json", REFERENCE_DP, 2.0).is_err());
        assert!(import_layout("{}", REFERENCE_DP, 2.0).is_err());
    }
}
