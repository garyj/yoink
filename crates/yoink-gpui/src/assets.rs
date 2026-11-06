//! Icons: the component library's default set plus the few extra icons the
//! window uses, instead of the full 7 MB catalog.

use std::borrow::Cow;

use gpui_kit::assets::{Assets, icon_assets};
use gpui_kit::{AssetSource, Result, SharedString};

icon_assets!(Extra, [Type, FileText, Link, ClipboardList, SearchX]);

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match Extra.load(path)? {
            Some(data) => Ok(Some(data)),
            None => Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = Extra.list(path)?;
        paths.extend(Assets.list(path)?);
        Ok(paths)
    }
}
