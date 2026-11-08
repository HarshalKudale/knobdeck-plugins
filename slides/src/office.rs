//! PowerPoint (Windows: COM automation of the running `PowerPoint.Application`; macOS: its
//! AppleScript dictionary) and Keynote (macOS: AppleScript). Each function acts on the running
//! app only and never starts it.

#[cfg(target_os = "macos")]
use knobdeck_sdk::{Error, Result};

/// What a slide command can do. Slide numbers are 1-based.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Next,
    Previous,
    Start,
    Stop,
    Goto(i64),
    Black,
    White,
    Resume,
}

#[allow(dead_code)] // each OS uses some of these
pub const PPT_NOT_RUNNING: &str = "PowerPoint isn't running";
#[allow(dead_code)]
pub const NO_SHOW: &str = "No slide show is running";

#[cfg(windows)]
pub mod powerpoint {
    //! Late-bound COM (IDispatch), the same object model as PowerPoint's VBA:
    //! `Application.SlideShowWindows(1).View.Next()` etc.
    use super::{Op, NO_SHOW, PPT_NOT_RUNNING};
    use knobdeck_sdk::{Error, Result};
    use windows::core::{IUnknown, Interface, GUID, PCWSTR, VARIANT};
    use windows::Win32::System::Com::{CLSIDFromProgID, CoInitializeEx, IDispatch, COINIT_MULTITHREADED, DISPATCH_FLAGS, DISPATCH_METHOD, DISPATCH_PROPERTYGET, DISPATCH_PROPERTYPUT, DISPPARAMS};
    use windows::Win32::System::Ole::{GetActiveObject, DISPID_PROPERTYPUT};

    const LOCALE_USER_DEFAULT: u32 = 0x0400;
    // PpSlideShowState
    const RUNNING: i32 = 1;
    const BLACK: i32 = 3;
    const WHITE: i32 = 4;

    fn werr(what: &str, e: windows::core::Error) -> Error {
        knobdeck_sdk::log(format!("PowerPoint {what}: {e}"));
        Error::msg(format!("PowerPoint did not respond ({what})"))
    }

    fn app() -> Result<IDispatch> {
        unsafe {
            // Each worker thread joins the multithreaded apartment once (repeat calls are no-ops).
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let clsid: GUID = CLSIDFromProgID(windows::core::w!("PowerPoint.Application")).map_err(|_| Error::msg(PPT_NOT_RUNNING))?;
            let mut unk: Option<IUnknown> = None;
            GetActiveObject(&clsid, None, &mut unk).map_err(|_| Error::msg(PPT_NOT_RUNNING))?;
            unk.ok_or_else(|| Error::msg(PPT_NOT_RUNNING))?.cast::<IDispatch>().map_err(|e| werr("connect", e))
        }
    }

    fn invoke(obj: &IDispatch, name: &str, flags: DISPATCH_FLAGS, mut args: Vec<VARIANT>) -> Result<VARIANT> {
        unsafe {
            let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
            let names = [PCWSTR(wide.as_ptr())];
            let mut id = 0i32;
            obj.GetIDsOfNames(&GUID::zeroed(), names.as_ptr(), 1, LOCALE_USER_DEFAULT, &mut id).map_err(|e| werr(name, e))?;
            // DISPPARAMS takes arguments last-first.
            args.reverse();
            let mut put = DISPID_PROPERTYPUT;
            let params = DISPPARAMS {
                rgvarg: if args.is_empty() { std::ptr::null_mut() } else { args.as_mut_ptr() },
                rgdispidNamedArgs: if flags == DISPATCH_PROPERTYPUT { &mut put } else { std::ptr::null_mut() },
                cArgs: args.len() as u32,
                cNamedArgs: (flags == DISPATCH_PROPERTYPUT) as u32,
            };
            let mut out = VARIANT::default();
            obj.Invoke(id, &GUID::zeroed(), LOCALE_USER_DEFAULT, flags, &params, Some(&mut out), None, None).map_err(|e| werr(name, e))?;
            Ok(out)
        }
    }

    fn get(obj: &IDispatch, name: &str, args: Vec<VARIANT>) -> Result<VARIANT> {
        invoke(obj, name, DISPATCH_PROPERTYGET | DISPATCH_METHOD, args)
    }

    fn object(v: &VARIANT, what: &str) -> Result<IDispatch> {
        IUnknown::try_from(v).and_then(|u| u.cast::<IDispatch>()).map_err(|e| werr(what, e))
    }

    fn int(v: &VARIANT) -> i64 {
        i32::try_from(v).map(i64::from).unwrap_or(0)
    }

    /// `SlideShowWindows(1)` when a show is running.
    fn show_window(app: &IDispatch) -> Result<Option<IDispatch>> {
        let windows = object(&get(app, "SlideShowWindows", vec![])?, "SlideShowWindows")?;
        if int(&get(&windows, "Count", vec![])?) < 1 {
            return Ok(None);
        }
        Ok(Some(object(&get(&windows, "Item", vec![VARIANT::from(1i32)])?, "SlideShowWindow")?))
    }

    fn view() -> Result<IDispatch> {
        let w = show_window(&app()?)?.ok_or_else(|| Error::msg(NO_SHOW))?;
        object(&get(&w, "View", vec![])?, "View")
    }

    pub fn running() -> bool {
        app().is_ok()
    }

    pub fn show_running() -> bool {
        app().and_then(|a| show_window(&a)).is_ok_and(|w| w.is_some())
    }

    pub fn run(op: Op) -> Result<()> {
        match op {
            Op::Start => {
                let a = app()?;
                let pres = object(&get(&a, "ActivePresentation", vec![]).map_err(|_| Error::msg("Open a presentation in PowerPoint first"))?, "ActivePresentation")?;
                let settings = object(&get(&pres, "SlideShowSettings", vec![])?, "SlideShowSettings")?;
                invoke(&settings, "Run", DISPATCH_METHOD, vec![]).map(|_| ())
            }
            Op::Next => invoke(&view()?, "Next", DISPATCH_METHOD, vec![]).map(|_| ()),
            Op::Previous => invoke(&view()?, "Previous", DISPATCH_METHOD, vec![]).map(|_| ()),
            Op::Stop => invoke(&view()?, "Exit", DISPATCH_METHOD, vec![]).map(|_| ()),
            Op::Goto(n) => invoke(&view()?, "GotoSlide", DISPATCH_METHOD, vec![VARIANT::from(n as i32)]).map(|_| ()),
            Op::Black | Op::White | Op::Resume => {
                let v = view()?;
                let state = match op {
                    Op::Black => BLACK,
                    Op::White => WHITE,
                    _ => RUNNING,
                };
                invoke(&v, "State", DISPATCH_PROPERTYPUT, vec![VARIANT::from(state)]).map(|_| ())
            }
        }
    }

    /// (current slide, slide count) while a show runs.
    pub fn position() -> Result<Option<(i64, i64)>> {
        let a = app()?;
        let Some(w) = show_window(&a)? else { return Ok(None) };
        let v = object(&get(&w, "View", vec![])?, "View")?;
        let cur = int(&get(&v, "CurrentShowPosition", vec![])?);
        let pres = object(&get(&w, "Presentation", vec![])?, "Presentation")?;
        let slides = object(&get(&pres, "Slides", vec![])?, "Slides")?;
        Ok(Some((cur, int(&get(&slides, "Count", vec![])?))))
    }

    /// Whether the show is blanked: Some(true) black, Some(false) white, None showing.
    pub fn blank() -> Result<Option<bool>> {
        let v = view()?;
        Ok(match i32::try_from(&get(&v, "State", vec![])?).unwrap_or(RUNNING) {
            BLACK => Some(true),
            WHITE => Some(false),
            _ => None,
        })
    }

}

#[cfg(target_os = "macos")]
fn osascript(app: &str, script: &str) -> Result<String> {
    // Only talk to an app that is already open (never launch it).
    let full = format!("if application \"{app}\" is running then\ntell application \"{app}\"\n{script}\nend tell\nelse\nreturn \"__not_running__\"\nend if");
    let out = std::process::Command::new("osascript").args(["-e", &full]).output().map_err(|e| Error::msg(format!("osascript failed: {e}")))?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        knobdeck_sdk::log(format!("{app}: {err}"));
        if err.contains("-1743") {
            return Err(Error::msg(format!("Allow KnobDeck to control {app} in System Settings ▸ Privacy & Security ▸ Automation")));
        }
        return Err(Error::msg(format!("{app} did not respond")));
    }
    if text == "__not_running__" {
        return Err(Error::msg(format!("{app} isn't running")));
    }
    Ok(text)
}

#[cfg(target_os = "macos")]
fn app_running(bundle: &str) -> bool {
    std::process::Command::new("osascript").args(["-e", &format!("application id \"{bundle}\" is running")]).output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
}

#[cfg(target_os = "macos")]
fn pair(text: &str) -> Option<(i64, i64)> {
    let (a, b) = text.split_once('/')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

#[cfg(target_os = "macos")]
pub mod powerpoint {
    use super::{osascript, pair, Op, NO_SHOW};
    use knobdeck_sdk::{Error, Result};

    const APP: &str = "Microsoft PowerPoint";
    const VIEW: &str = "slide show view of slide show window 1";

    fn in_show(script: &str) -> Result<String> {
        let r = osascript(APP, &format!("if (count of slide show windows) is 0 then return \"__no_show__\"\n{script}"))?;
        if r == "__no_show__" {
            return Err(Error::msg(NO_SHOW));
        }
        Ok(r)
    }

    pub fn running() -> bool {
        super::app_running("com.microsoft.Powerpoint")
    }

    pub fn show_running() -> bool {
        running() && osascript(APP, "return (count of slide show windows) > 0").is_ok_and(|r| r == "true")
    }

    pub fn run(op: Op) -> Result<()> {
        match op {
            Op::Start => osascript(APP, "if (count of presentations) is 0 then error \"no presentation\"\nrun slide show slide show settings of active presentation").map_err(|e| if e.message().contains("did not respond") { Error::msg("Open a presentation in PowerPoint first") } else { e }),
            Op::Next => in_show(&format!("go to next slide {VIEW}")),
            Op::Previous => in_show(&format!("go to previous slide {VIEW}")),
            Op::Stop => in_show(&format!("exit slide show {VIEW}")),
            Op::Goto(n) => in_show(&format!("go to slide {VIEW} number {n}")),
            Op::Black => in_show(&format!("set slide state of {VIEW} to slide show state black screen")),
            Op::White => in_show(&format!("set slide state of {VIEW} to slide show state white screen")),
            Op::Resume => in_show(&format!("set slide state of {VIEW} to slide show state running")),
        }
        .map(|_| ())
    }

    pub fn position() -> Result<Option<(i64, i64)>> {
        match in_show(&format!("return ((current show position of {VIEW}) as text) & \"/\" & ((count of slides of active presentation) as text)")) {
            Ok(t) => Ok(pair(&t)),
            Err(e) if e.message() == NO_SHOW => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn blank() -> Result<Option<bool>> {
        let s = in_show(&format!("return (slide state of {VIEW}) as text"))?;
        Ok(if s.contains("black") {
            Some(true)
        } else if s.contains("white") {
            Some(false)
        } else {
            None
        })
    }
}

#[cfg(target_os = "macos")]
pub mod keynote {
    use super::{osascript, pair, Op, NO_SHOW};
    use knobdeck_sdk::{Error, Result};

    const APP: &str = "Keynote";

    fn front() -> Result<()> {
        if osascript(APP, "return (count of documents) > 0")? != "true" {
            return Err(Error::msg("Open a presentation in Keynote first"));
        }
        Ok(())
    }

    fn in_show(script: &str) -> Result<String> {
        let r = osascript(APP, &format!("if not playing then return \"__no_show__\"\n{script}"))?;
        if r == "__no_show__" {
            return Err(Error::msg(NO_SHOW));
        }
        Ok(r)
    }

    pub fn running() -> bool {
        super::app_running("com.apple.iWork.Keynote")
    }

    pub fn show_running() -> bool {
        running() && osascript(APP, "return playing").is_ok_and(|r| r == "true")
    }

    pub fn run(op: Op) -> Result<()> {
        match op {
            Op::Start => {
                front()?;
                osascript(APP, "start front document from first slide of front document")
            }
            Op::Next => in_show("show next"),
            Op::Previous => in_show("show previous"),
            Op::Stop => in_show("stop front document"),
            // Starting from a slide while playing moves the show there.
            Op::Goto(n) => in_show(&format!("if {n} > (count of slides of front document) then error \"no slide\"\nstart front document from slide {n} of front document")),
            Op::Black | Op::White | Op::Resume => return Err(Error::msg("Keynote has no command to blank the screen")),
        }
        .map(|_| ())
    }

    pub fn position() -> Result<Option<(i64, i64)>> {
        match in_show("return ((slide number of current slide of front document) as text) & \"/\" & ((count of slides of front document) as text)") {
            Ok(t) => Ok(pair(&t)),
            Err(e) if e.message() == NO_SHOW => Ok(None),
            Err(e) => Err(e),
        }
    }
}
