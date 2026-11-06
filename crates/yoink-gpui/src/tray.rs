//! StatusNotifierItem tray. A native SNI item receives Activate on left
//! click, which libappindicator-style menu-only trays cannot do.

use crate::ipc::Request;

struct Tray {
    send: Box<dyn Fn(Request) + Send>,
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "yoink".into()
    }

    fn title(&self) -> String {
        "yoink".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        static ICON_PNG: &[u8] = include_bytes!("../../../src-tauri/icons/32x32.png");
        let Ok(decoded) = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png)
        else {
            return Vec::new();
        };
        let rgba = decoded.to_rgba8();
        let (width, height) = (rgba.width() as i32, rgba.height() as i32);
        let mut data = rgba.into_raw();
        // RGBA to ARGB32 in network byte order, per the SNI spec.
        for pixel in data.as_chunks_mut::<4>().0 {
            pixel.rotate_right(1);
        }
        vec![ksni::Icon {
            width,
            height,
            data,
        }]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        (self.send)(Request::Toggle);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: "Show / Hide".into(),
                activate: Box::new(|tray: &mut Self| (tray.send)(Request::Toggle)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|tray: &mut Self| (tray.send)(Request::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

pub fn spawn(send: impl Fn(Request) + Send + 'static) {
    use ksni::blocking::TrayMethods;
    let tray = Tray {
        send: Box::new(send),
    };
    if let Err(error) = tray.spawn() {
        eprintln!("yoink: failed to create tray icon: {error}");
    }
}
