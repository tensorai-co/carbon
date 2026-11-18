//! A running machine's desktop, for computer use: its screen, input as a user's, its windows and clipboard.

use carbon::Button;
use serde_json::json;

use super::{Is, Tool, prop};
use crate::shape::{self, Args, Failure, Outcome};
use crate::{Progress, Server};

pub(crate) fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "screenshot",
            title: "Take a screenshot",
            description: "A running machine's screen, as an image: its desktop, with every window its programs show (Linux's on Wayland and X11, Windows').",
            properties: json!({}),
            required: &[],
            is: Is::ReadOnly,
            on_machine: true,
            host: false,
            run: screenshot,
        },
        Tool {
            name: "screen_input",
            title: "Use the screen",
            description: "Input on a running machine's screen, as a user's: click, double_click, right_click or move at x,y; drag from x,y to to_x,to_y; scroll at x,y by notches (negative is up); type text; or press a key combination (ctrl+s, Return, alt+F4). Coordinates are the screenshot's pixels.",
            properties: json!({
                "action": { "type": "string", "enum": ["click", "double_click", "right_click", "middle_click", "move", "drag", "scroll", "type", "key"], "description": "What to do" },
                "x": prop("integer", "Where, across"),
                "y": prop("integer", "Where, down"),
                "to_x": prop("integer", "A drag's end, across"),
                "to_y": prop("integer", "A drag's end, down"),
                "notches": prop("integer", "How far to scroll: positive down, negative up"),
                "text": prop("string", "What to type"),
                "key": prop("string", "A key or combination: Return, Tab, ctrl+c, shift+Tab, alt+F4"),
            }),
            required: &["action"],
            is: Is::Changes,
            on_machine: true,
            host: false,
            run: input,
        },
        Tool {
            name: "list_windows",
            title: "List windows",
            description: "A running machine's windows, top first: id, place, size, state and title.",
            properties: json!({}),
            required: &[],
            is: Is::ReadOnly,
            on_machine: true,
            host: false,
            run: windows,
        },
        Tool {
            name: "clipboard",
            title: "The clipboard",
            description: "A running machine's clipboard text; with text, sets it, as if copied.",
            properties: json!({ "text": prop("string", "What to put on it") }),
            required: &[],
            is: Is::Changes,
            on_machine: true,
            host: false,
            run: clipboard,
        },
        Tool {
            name: "resize_screen",
            title: "Resize the screen",
            description: "Changes a running machine's screen size, and its scale (2 for a high-density screen).",
            properties: json!({ "width": prop("integer", "Pixels across"), "height": prop("integer", "Pixels down"), "scale": prop("number", "1 by default") }),
            required: &["width", "height"],
            is: Is::Idempotent,
            on_machine: true,
            host: false,
            run: resize,
        },
    ]
}

fn screen(server: &Server, args: &Args) -> Result<carbon::Screen, Failure> {
    let machine = server.machine(args)?;
    if !machine.is_running() {
        return Err(Failure(format!("{} isn't running", machine.name())));
    }
    Ok(machine.screen())
}

fn screenshot(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let png = screen(server, args)?.screenshot()?;
    Ok(json!({ "content": [{ "type": "image", "data": shape::base64(&png), "mimeType": "image/png" }], "isError": false }))
}

fn at(args: &Args) -> Result<(i32, i32), Failure> {
    match (args.i64("x"), args.i64("y")) {
        (Some(x), Some(y)) => Ok((x as i32, y as i32)),
        _ => Err(Failure("x and y are needed".into())),
    }
}

fn input(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let s = screen(server, args)?;
    let action = args.str("action")?;
    match action.as_str() {
        "click" => {
            let (x, y) = at(args)?;
            s.click_with(x, y, Button::Left, 1)?;
        }
        "double_click" => {
            let (x, y) = at(args)?;
            s.click_with(x, y, Button::Left, 2)?;
        }
        "right_click" => {
            let (x, y) = at(args)?;
            s.click_with(x, y, Button::Right, 1)?;
        }
        "middle_click" => {
            let (x, y) = at(args)?;
            s.click_with(x, y, Button::Middle, 1)?;
        }
        "move" => {
            let (x, y) = at(args)?;
            s.move_to(x, y)?;
        }
        "drag" => {
            let from = at(args)?;
            let to = match (args.i64("to_x"), args.i64("to_y")) {
                (Some(x), Some(y)) => (x as i32, y as i32),
                _ => return Err(Failure("a drag needs to_x and to_y".into())),
            };
            s.drag(from, to)?;
        }
        "scroll" => {
            let (x, y) = at(args)?;
            s.scroll(x, y, args.i64("notches").unwrap_or(3) as i32, 0)?;
        }
        "type" => s.type_text(&args.str("text")?)?,
        "key" => s.key(&args.str("key")?)?,
        other => return Err(Failure(format!("{other} isn't an action: click, double_click, right_click, move, drag, scroll, type, key"))),
    }
    Ok(shape::text(format!("{action} done; take a screenshot to see what it did")))
}

fn windows(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let list: Vec<_> = screen(server, args)?
        .windows()?
        .iter()
        .map(|w| json!({ "id": w.id, "x": w.x, "y": w.y, "width": w.width, "height": w.height, "states": w.states, "title": w.title }))
        .collect();
    Ok(shape::data(json!({ "windows": list })))
}

fn clipboard(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let s = screen(server, args)?;
    match args.opt_str("text") {
        Some(t) => {
            s.set_clipboard(&t)?;
            Ok(shape::text("set"))
        }
        None => Ok(shape::text(s.clipboard()?)),
    }
}

fn resize(server: &Server, args: &Args, _: &Progress) -> Outcome {
    let (w, h) = match (args.i64("width"), args.i64("height")) {
        (Some(w), Some(h)) => (w as i32, h as i32),
        _ => return Err(Failure("width and height are needed".into())),
    };
    screen(server, args)?.resize(w, h, args.f64("scale").unwrap_or(1.0) as f32)?;
    Ok(shape::text(format!("{w}x{h}")))
}
