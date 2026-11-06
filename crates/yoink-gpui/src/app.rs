//! The window: a search field, the history list, and a preview of the
//! selected item.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, Theme};
use gpui_kit::{prelude::*, *};
use yoink_clipboard as clipboard;
use yoink_core::{HistoryStore, ItemKind};

use crate::ipc::Request;
use crate::text;
use crate::x11;
use gpui_kit::assets::IconName as Icons;

gpui_kit::actions!(
    yoink,
    [
        SelectPrev, SelectNext, PagePrev, PageNext, Confirm, Dismiss, DeleteItem
    ]
);

const CONTEXT: &str = "Yoink";
const ROW_HEIGHT: f32 = 44.;
const PAGE_ROWS: usize = 8;
const LIST_WIDTH: f32 = 340.;

pub fn window_size() -> Size<Pixels> {
    size(px(880.), px(540.))
}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", SelectPrev, Some(CONTEXT)),
        KeyBinding::new("down", SelectNext, Some(CONTEXT)),
        KeyBinding::new("pageup", PagePrev, Some(CONTEXT)),
        KeyBinding::new("pagedown", PageNext, Some(CONTEXT)),
        KeyBinding::new("enter", Confirm, Some(CONTEXT)),
        KeyBinding::new("escape", Dismiss, Some(CONTEXT)),
        // Same depth as the input's own bindings and registered later, so it wins.
        KeyBinding::new("shift-delete", DeleteItem, Some("Yoink > Input")),
    ]);
}

pub enum Message {
    Request(Request),
    HistoryChanged,
}

enum Kind {
    Text {
        lines: usize,
    },
    Link,
    Color(u32),
    Image {
        thumb: Arc<Image>,
        width: u32,
        height: u32,
    },
}

struct Row {
    id: i64,
    kind: Kind,
    title: SharedString,
    highlights: Vec<Range<usize>>,
    copied_at_ms: i64,
    first_copied_at_ms: i64,
}

enum Preview {
    Empty,
    Text {
        id: i64,
        body: SharedString,
        highlights: Vec<Range<usize>>,
        chars: usize,
        words: usize,
        lines: usize,
        shown_chars: usize,
    },
    Image {
        id: i64,
        image: Option<Arc<Image>>,
        bytes: usize,
    },
}

pub struct Yoink {
    store: Arc<Mutex<HistoryStore>>,
    clipboard: mpsc::Sender<clipboard::Write>,
    x11: mpsc::Sender<x11::Command>,
    search: Entity<InputState>,
    rows: Vec<Row>,
    total: usize,
    selected: usize,
    list_scroll: UniformListScrollHandle,
    preview: Preview,
    preview_scroll: ScrollHandle,
    shown: bool,
    confirming_clear: bool,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl Yoink {
    pub fn new(
        store: Arc<Mutex<HistoryStore>>,
        clipboard: mpsc::Sender<clipboard::Write>,
        x11: mpsc::Sender<x11::Command>,
        shown: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search clipboard history"));
        let subscriptions = vec![
            cx.subscribe_in(&search, window, |this, _, event: &InputEvent, _, cx| {
                if let InputEvent::Change = event {
                    this.reload(cx);
                    this.select(0, cx);
                }
            }),
            cx.observe_window_appearance(window, |_, window, cx| {
                apply_theme(Some(window), cx);
            }),
        ];
        search.update(cx, |search, cx| search.focus(window, cx));
        let mut this = Self {
            store,
            clipboard,
            x11,
            search,
            rows: Vec::new(),
            total: 0,
            selected: 0,
            list_scroll: UniformListScrollHandle::new(),
            preview: Preview::Empty,
            preview_scroll: ScrollHandle::new(),
            shown,
            confirming_clear: false,
            error: None,
            _subscriptions: subscriptions,
        };
        this.reload(cx);
        this.select(0, cx);
        this
    }

    pub fn handle(&mut self, message: Message, window: &mut Window, cx: &mut Context<Self>) {
        match message {
            Message::Request(Request::Toggle) if self.shown && window.is_window_active() => {
                self.hide(window, cx)
            }
            Message::Request(Request::Toggle | Request::Show) => self.show(window, cx),
            Message::Request(Request::Hide) => self.hide(window, cx),
            Message::Request(Request::Quit) => cx.quit(),
            Message::Request(Request::Ping) => {}
            Message::HistoryChanged => {
                let selected = self.rows.get(self.selected).map(|row| row.id);
                self.reload(cx);
                let index = selected
                    .and_then(|id| self.rows.iter().position(|row| row.id == id))
                    .unwrap_or(0);
                self.select(index, cx);
            }
        }
    }

    fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.shown {
            self.shown = true;
            self.confirming_clear = false;
            self.reload(cx);
            self.select(0, cx);
            self.search.update(cx, |search, cx| {
                search.select_all(window, cx);
                search.focus(window, cx);
            });
        }
        self.send_window(window, |window, size| x11::Command::Show {
            window,
            width: size.width,
            height: size.height,
        });
    }

    pub fn hide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.shown = false;
        self.error = None;
        cx.notify();
        self.send_window(window, |window, _| x11::Command::Hide { window });
    }

    fn send_window(&self, window: &Window, command: impl FnOnce(u32, Size<u32>) -> x11::Command) {
        let Some(id) = x11::window_id(window) else {
            return;
        };
        let scale = window.scale_factor();
        let bounds = window.bounds().size;
        let physical = size(
            (f32::from(bounds.width) * scale) as u32,
            (f32::from(bounds.height) * scale) as u32,
        );
        let _ = self.x11.send(command(id, physical));
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value().to_string();
        // Reusing an Arc<Image> keeps GPUI's decoded copy and skips rehashing.
        let thumbs: HashMap<i64, Arc<Image>> = self
            .rows
            .iter()
            .filter_map(|row| match &row.kind {
                Kind::Image { thumb, .. } => Some((row.id, thumb.clone())),
                _ => None,
            })
            .collect();
        let result = (|| {
            let store = self.store.lock().unwrap();
            let total = store.count_matches(&query)?;
            let items = store.preview_page(&query, store.max_items(), 0)?;
            let rows = items
                .into_iter()
                .map(|item| {
                    let kind = match item.kind {
                        ItemKind::Image => {
                            let thumb = match thumbs.get(&item.id) {
                                Some(thumb) => thumb.clone(),
                                None => Arc::new(Image::from_bytes(
                                    ImageFormat::Png,
                                    store.thumbnail(item.id)?.unwrap_or_default(),
                                )),
                            };
                            Kind::Image {
                                thumb,
                                width: item.width.unwrap_or(0),
                                height: item.height.unwrap_or(0),
                            }
                        }
                        ItemKind::Text => match text::hex_color(&item.text) {
                            Some(rgb) => Kind::Color(rgb),
                            None if text::is_link(&item.text) => Kind::Link,
                            None => Kind::Text {
                                lines: item.text.trim_end().lines().count().max(1),
                            },
                        },
                    };
                    let title = match &kind {
                        Kind::Image { width, height, .. } => format!("Image {width} × {height}"),
                        _ => text::title(&item.text),
                    };
                    Ok(Row {
                        id: item.id,
                        highlights: text::match_ranges(&title, &query),
                        title: title.into(),
                        kind,
                        copied_at_ms: item.last_copied_at_ms,
                        first_copied_at_ms: item.created_at_ms,
                    })
                })
                .collect::<yoink_core::Result<Vec<_>>>()?;
            yoink_core::Result::Ok((total, rows))
        })();
        match result {
            Ok((total, rows)) => {
                self.total = total;
                self.rows = rows;
            }
            Err(error) => self.error = Some(format!("Cannot load history: {error}").into()),
        }
        cx.notify();
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = index.min(self.rows.len().saturating_sub(1));
        self.list_scroll
            .scroll_to_item(self.selected, ScrollStrategy::Nearest);
        self.load_preview(cx);
        cx.notify();
    }

    fn load_preview(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(self.selected) else {
            self.preview = Preview::Empty;
            return;
        };
        let id = row.id;
        match (&self.preview, &row.kind) {
            (Preview::Text { id: shown, .. } | Preview::Image { id: shown, .. }, _)
                if *shown == id => {}
            (_, Kind::Image { thumb, .. }) => {
                let thumb = thumb.clone();
                self.preview = Preview::Image {
                    id,
                    image: None,
                    bytes: 0,
                };
                self.preview_scroll = ScrollHandle::new();
                let store = self.store.clone();
                cx.spawn(async move |this, cx| {
                    let loaded = cx
                        .background_spawn(async move {
                            let data = store.lock().unwrap().image_data(id).ok()??;
                            let bytes = data.len();
                            Some((Arc::new(Image::from_bytes(ImageFormat::Png, data)), bytes))
                        })
                        .await;
                    this.update(cx, |this, cx| {
                        if let Preview::Image {
                            id: shown,
                            image,
                            bytes,
                        } = &mut this.preview
                            && *shown == id
                        {
                            *image = Some(loaded.as_ref().map_or(thumb, |(full, _)| full.clone()));
                            *bytes = loaded.map_or(0, |(_, bytes)| bytes);
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .detach();
            }
            _ => {
                let query = self.search.read(cx).value().to_string();
                let full = self.store.lock().unwrap().get(id).ok().flatten();
                let full = full.map(|item| item.text).unwrap_or_default();
                let chars = full.chars().count();
                let body: String = full.chars().take(text::PREVIEW_CHARS).collect();
                self.preview = Preview::Text {
                    id,
                    highlights: text::match_ranges(&body, &query),
                    shown_chars: body.chars().count(),
                    lines: full.trim_end().lines().count().max(1),
                    words: full.split_whitespace().count(),
                    chars,
                    body: body.into(),
                };
                self.preview_scroll = ScrollHandle::new();
            }
        }
    }

    fn select_prev(&mut self, _: &SelectPrev, _: &mut Window, cx: &mut Context<Self>) {
        self.select(self.selected.saturating_sub(1), cx);
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.select(self.selected + 1, cx);
    }

    fn page_prev(&mut self, _: &PagePrev, _: &mut Window, cx: &mut Context<Self>) {
        self.select(self.selected.saturating_sub(PAGE_ROWS), cx);
    }

    fn page_next(&mut self, _: &PageNext, _: &mut Window, cx: &mut Context<Self>) {
        self.select(self.selected + PAGE_ROWS, cx);
    }

    fn dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        self.hide(window, cx);
    }

    fn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selected(cx);
    }

    fn copy_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.rows.get(self.selected).map(|row| row.id) else {
            return;
        };
        let store = self.store.clone();
        let clipboard = self.clipboard.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { copy_item(&store, &clipboard, id) })
                .await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(()) => {
                    this.search
                        .update(cx, |search, cx| search.set_value("", window, cx));
                    this.hide(window, cx);
                }
                Err(error) => {
                    this.error = Some(format!("Cannot copy item: {error}").into());
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn delete_selected(&mut self, _: &DeleteItem, _: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.rows.get(self.selected).map(|row| row.id) else {
            return;
        };
        if let Err(error) = self.store.lock().unwrap().remove(id) {
            self.error = Some(format!("Cannot remove item: {error}").into());
        }
        self.reload(cx);
        self.select(self.selected, cx);
    }

    fn clear_history(&mut self, cx: &mut Context<Self>) {
        if !self.confirming_clear {
            self.confirming_clear = true;
            cx.notify();
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(4)).await;
                this.update(cx, |this, cx| {
                    this.confirming_clear = false;
                    cx.notify();
                })
                .ok();
            })
            .detach();
            return;
        }
        self.confirming_clear = false;
        if let Err(error) = self.store.lock().unwrap().clear() {
            self.error = Some(format!("Cannot clear history: {error}").into());
        }
        self.reload(cx);
        self.select(0, cx);
    }
}

fn copy_item(
    store: &Mutex<HistoryStore>,
    clipboard: &mpsc::Sender<clipboard::Write>,
    id: i64,
) -> Result<(), String> {
    let (item, png) = {
        let store = store.lock().unwrap();
        let item = store
            .get(id)
            .map_err(|e| e.to_string())?
            .ok_or("item no longer exists")?;
        let png = match item.kind {
            ItemKind::Text => None,
            ItemKind::Image => Some(
                store
                    .image_data(id)
                    .map_err(|e| e.to_string())?
                    .ok_or("image data missing")?,
            ),
        };
        (item, png)
    };
    let content = match png {
        None => clipboard::Content::Text(item.text),
        Some(png) => clipboard::decode_image(png)?,
    };
    let (reply, result) = mpsc::channel();
    clipboard
        .send(clipboard::Write { content, reply })
        .map_err(|_| "clipboard worker is unavailable")?;
    result
        .recv()
        .map_err(|_| "clipboard worker stopped before completing the copy")?
}

/// Colors that the component theme does not cover.
struct Palette {
    background: Hsla,
    panel: Hsla,
    border: Hsla,
    text: Hsla,
    muted: Hsla,
    faint: Hsla,
    accent: Hsla,
    selected: Hsla,
    hover: Hsla,
    chip: Hsla,
    danger: Hsla,
}

impl Palette {
    fn new(dark: bool) -> Self {
        let accent = rgb(0xff9f1a).into();
        if dark {
            Self {
                background: rgb(0x16171b).into(),
                panel: rgb(0x1b1c21).into(),
                border: hsla(0., 0., 1., 0.07),
                text: rgb(0xeceef2).into(),
                muted: rgb(0x9a9ca8).into(),
                faint: rgb(0x6b6e7a).into(),
                accent,
                selected: hsla(35. / 360., 1., 0.55, 0.14),
                hover: hsla(0., 0., 1., 0.04),
                chip: hsla(0., 0., 1., 0.06),
                danger: rgb(0xff6b62).into(),
            }
        } else {
            Self {
                background: rgb(0xfbfbfc).into(),
                panel: rgb(0xffffff).into(),
                border: hsla(0., 0., 0., 0.08),
                text: rgb(0x1c1d22).into(),
                muted: rgb(0x62656f).into(),
                faint: rgb(0x9a9ca6).into(),
                accent: rgb(0xf08a00).into(),
                selected: hsla(35. / 360., 1., 0.5, 0.13),
                hover: hsla(0., 0., 0., 0.035),
                chip: hsla(0., 0., 0., 0.05),
                danger: rgb(0xd93a31).into(),
            }
        }
    }
}

pub fn apply_theme(window: Option<&mut Window>, cx: &mut App) {
    Theme::sync_system_appearance(window, cx);
    Theme::update(cx, |theme| {
        let palette = Palette::new(theme.mode.is_dark());
        theme.font_size = px(15.);
        theme.background = palette.background;
        theme.foreground = palette.text;
        theme.caret = palette.accent;
        theme.selection = palette.accent.opacity(0.3);
        theme.ring = palette.accent;
        theme.muted_foreground = palette.faint;
    });
}

impl Render for Yoink {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::new(cx.theme().mode.is_dark());
        div()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::page_prev))
            .on_action(cx.listener(Self::page_next))
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::delete_selected))
            .size_full()
            .flex()
            .flex_col()
            .bg(palette.background)
            .text_color(palette.text)
            .border_1()
            .border_color(palette.border)
            .child(self.render_header(&palette, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .border_t_1()
                    .border_color(palette.border)
                    .map(|body| {
                        if self.rows.is_empty() {
                            body.child(self.render_empty(&palette, cx))
                        } else {
                            body.child(self.render_list(&palette, cx))
                                .child(self.render_preview(&palette, cx))
                        }
                    }),
            )
            .when_some(self.error.clone(), |root, error| {
                root.child(
                    div()
                        .px_4()
                        .py_2()
                        .border_t_1()
                        .border_color(palette.border)
                        .text_sm()
                        .text_color(palette.danger)
                        .child(error),
                )
            })
            .child(self.render_footer(&palette, cx))
    }
}

impl Yoink {
    fn render_header(&self, palette: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let count = if self.search.read(cx).value().is_empty() {
            format!(
                "{} {}",
                text::thousands(self.total),
                plural(self.total, "item", "items")
            )
        } else {
            format!(
                "{} {}",
                text::thousands(self.total),
                plural(self.total, "match", "matches")
            )
        };
        div()
            .h(px(60.))
            .flex_none()
            .flex()
            .items_center()
            .gap_3()
            .px_5()
            .child(
                Icon::new(Icons::Search)
                    .size(px(18.))
                    .text_color(palette.muted),
            )
            .child(
                div().flex_1().child(
                    Input::new(&self.search)
                        .appearance(false)
                        .large()
                        .cleanable(false),
                ),
            )
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(palette.faint)
                    .child(count),
            )
    }

    fn render_list(&self, palette: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let now = clipboard::now_ms();
        div()
            .relative()
            .w(px(LIST_WIDTH))
            .flex_none()
            .h_full()
            .border_r_1()
            .border_color(palette.border)
            .child(
                uniform_list(
                    "history",
                    self.rows.len(),
                    cx.processor(move |this, range: Range<usize>, _, cx| {
                        let palette = Palette::new(cx.theme().mode.is_dark());
                        range
                            .map(|index| this.render_row(index, now, &palette, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.list_scroll)
                .py_1p5()
                .size_full(),
            )
            .child(Scrollbar::vertical(&self.list_scroll))
    }

    fn render_row(
        &self,
        index: usize,
        now: i64,
        palette: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = &self.rows[index];
        let selected = index == self.selected;
        let hover = palette.hover;
        let title = StyledText::new(row.title.clone()).with_highlights(
            row.highlights.iter().cloned().map(|range| {
                (
                    range,
                    HighlightStyle {
                        color: Some(palette.accent),
                        font_weight: Some(FontWeight::SEMIBOLD),
                        ..Default::default()
                    },
                )
            }),
        );
        div()
            .id(index)
            .h(px(ROW_HEIGHT))
            .px_1p5()
            .child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_2p5()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .when(selected, |row| row.bg(palette.selected))
                    .when(!selected, |row| row.hover(move |style| style.bg(hover)))
                    .child(self.render_badge(row, selected, palette))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .text_color(if selected {
                                palette.text
                            } else {
                                palette.text.opacity(0.92)
                            })
                            .child(title),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(if selected {
                                palette.muted
                            } else {
                                palette.faint
                            })
                            .child(text::ago_short(now, row.copied_at_ms)),
                    ),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.select(index, cx);
                this.search
                    .update(cx, |search, cx| search.focus(window, cx));
                if event.click_count() >= 2 {
                    this.copy_selected(cx);
                }
            }))
            .into_any_element()
    }

    fn render_badge(&self, row: &Row, selected: bool, palette: &Palette) -> AnyElement {
        let frame = || {
            div()
                .size(px(26.))
                .flex_none()
                .rounded(px(6.))
                .overflow_hidden()
                .flex()
                .items_center()
                .justify_center()
        };
        let icon = |name: Icons| {
            frame()
                .bg(if selected {
                    palette.accent.opacity(0.18)
                } else {
                    palette.chip
                })
                .child(Icon::new(name).size(px(14.)).text_color(if selected {
                    palette.accent
                } else {
                    palette.muted
                }))
                .into_any_element()
        };
        match &row.kind {
            Kind::Text { lines: 1 } => icon(Icons::Type),
            Kind::Text { .. } => icon(Icons::FileText),
            Kind::Link => icon(Icons::Link),
            Kind::Color(rgb_value) => div()
                .size(px(26.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .size(px(18.))
                        .rounded_full()
                        .border_1()
                        .border_color(palette.border)
                        .bg(rgb(*rgb_value)),
                )
                .into_any_element(),
            Kind::Image { thumb, .. } => frame()
                .border_1()
                .border_color(palette.border)
                .child(img(thumb.clone()).size_full().object_fit(ObjectFit::Cover))
                .into_any_element(),
        }
    }

    fn render_preview(&self, palette: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let now = clipboard::now_ms();
        let Some(row) = self.rows.get(self.selected) else {
            return div().flex_1().into_any_element();
        };
        let mono = cx.theme().mono_font_family.clone();
        let mut facts: Vec<(&'static str, SharedString)> = Vec::new();
        let body =
            match (&row.kind, &self.preview) {
                (Kind::Color(value), _) => {
                    facts.push(("Type", "Color".into()));
                    facts.push((
                        "RGB",
                        format!(
                            "{}, {}, {}",
                            value >> 16 & 0xff,
                            value >> 8 & 0xff,
                            value & 0xff
                        )
                        .into(),
                    ));
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_4()
                        .child(
                            div()
                                .size(px(112.))
                                .rounded(px(24.))
                                .border_1()
                                .border_color(palette.border)
                                .shadow_lg()
                                .bg(rgb(*value)),
                        )
                        .child(
                            div()
                                .text_lg()
                                .font_family(mono.clone())
                                .child(row.title.clone()),
                        )
                        .into_any_element()
                }
                (
                    Kind::Image {
                        thumb,
                        width,
                        height,
                    },
                    Preview::Image { image, bytes, .. },
                ) => {
                    facts.push(("Type", "Image".into()));
                    facts.push(("Dimensions", format!("{width} × {height} pixels").into()));
                    if *bytes > 0 {
                        facts.push(("Size", human_bytes(*bytes).into()));
                    }
                    let image = image.clone().unwrap_or_else(|| thumb.clone());
                    div()
                        .size_full()
                        .p_6()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            img(image)
                                .max_w_full()
                                .max_h_full()
                                .object_fit(ObjectFit::Contain)
                                .rounded(px(6.)),
                        )
                        .into_any_element()
                }
                (
                    kind,
                    Preview::Text {
                        body,
                        highlights,
                        chars,
                        words,
                        lines,
                        shown_chars,
                        ..
                    },
                ) => {
                    let link = matches!(kind, Kind::Link);
                    facts.push(("Type", if link { "Link" } else { "Text" }.into()));
                    facts.push(("Characters", text::thousands(*chars).into()));
                    facts.push(("Words", text::thousands(*words).into()));
                    facts.push(("Lines", text::thousands(*lines).into()));
                    let monospace = *lines > 1;
                    let styled = StyledText::new(body.clone()).with_highlights(
                        highlights.iter().cloned().map(|range| {
                            (
                                range,
                                HighlightStyle {
                                    background_color: Some(palette.accent.opacity(0.28)),
                                    ..Default::default()
                                },
                            )
                        }),
                    );
                    div()
                        .id("preview")
                        .size_full()
                        .track_scroll(&self.preview_scroll)
                        .overflow_y_scroll()
                        .px_6()
                        .py_5()
                        .child(
                            div()
                                .w_full()
                                .when(monospace, |text| {
                                    text.font_family(mono.clone())
                                        .text_size(px(13.))
                                        .line_height(px(21.))
                                })
                                .when(!monospace, |text| {
                                    text.text_size(px(16.)).line_height(px(25.))
                                })
                                .when(link, |text| text.text_color(palette.accent))
                                .child(styled),
                        )
                        .when(shown_chars < chars, |body| {
                            body.child(div().pt_4().text_xs().text_color(palette.faint).child(
                                format!(
                                    "Showing the first {} of {} characters",
                                    text::thousands(*shown_chars),
                                    text::thousands(*chars)
                                ),
                            ))
                        })
                        .into_any_element()
                }
                _ => div().into_any_element(),
            };
        facts.push(("Last copied", text::ago_long(now, row.copied_at_ms).into()));
        if row.first_copied_at_ms < row.copied_at_ms - 60_000 {
            facts.push((
                "First copied",
                text::ago_long(now, row.first_copied_at_ms).into(),
            ));
        }

        let information = div()
            .flex_none()
            .px_6()
            .pt_3()
            .pb_2()
            .border_t_1()
            .border_color(palette.border)
            .child(
                div()
                    .pb_1()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(palette.muted)
                    .child("Information"),
            )
            .children(
                facts
                    .into_iter()
                    .enumerate()
                    .map(|(index, (label, value))| {
                        div()
                            .h(px(28.))
                            .flex()
                            .items_center()
                            .justify_between()
                            .when(index > 0, |fact| {
                                fact.border_t_1().border_color(palette.border)
                            })
                            .text_xs()
                            .child(div().text_color(palette.faint).child(label))
                            .child(div().text_color(palette.text.opacity(0.85)).child(value))
                    }),
            );

        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(palette.panel)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(body)
                    .when(matches!(self.preview, Preview::Text { .. }), |body| {
                        body.child(Scrollbar::vertical(&self.preview_scroll))
                    }),
            )
            .child(information)
            .into_any_element()
    }

    fn render_empty(&self, palette: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value();
        let (icon, title, description): (Icons, SharedString, SharedString) = if query.is_empty() {
            (
                Icons::ClipboardList,
                "Nothing yoinked yet".into(),
                "Copy text or an image in any app and it appears here.".into(),
            )
        } else {
            (
                Icons::SearchX,
                format!("No matches for \u{201c}{query}\u{201d}").into(),
                "Search looks at text items. Images are listed when the search is empty.".into(),
            )
        };
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .child(
                div()
                    .size(px(56.))
                    .rounded_full()
                    .bg(palette.chip)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Icon::new(icon).size(px(24.)).text_color(palette.muted)),
            )
            .child(
                div()
                    .text_base()
                    .font_weight(FontWeight::MEDIUM)
                    .child(title),
            )
            .child(div().text_sm().text_color(palette.faint).child(description))
    }

    fn render_footer(&self, palette: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let hint = |keys: &[&'static str], label: &'static str| {
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .children(keys.iter().map(|key| {
                    div()
                        .min_w(px(20.))
                        .h(px(20.))
                        .px_1p5()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.))
                        .bg(palette.chip)
                        .border_1()
                        .border_color(palette.border)
                        .text_xs()
                        .text_color(palette.muted)
                        .child(*key)
                }))
                .child(div().text_xs().text_color(palette.faint).child(label))
        };
        let danger = palette.danger;
        let muted = palette.faint;
        let has_items = !self.rows.is_empty() || self.total > 0;
        div()
            .h(px(42.))
            .flex_none()
            .flex()
            .items_center()
            .gap_5()
            .px_4()
            .border_t_1()
            .border_color(palette.border)
            .child(hint(&["↑", "↓"], "Navigate"))
            .child(hint(&["↵"], "Copy"))
            .child(hint(&["⇧", "Del"], "Delete"))
            .child(hint(&["Esc"], "Close"))
            .child(div().flex_1())
            .when(
                has_items && self.search.read(cx).value().is_empty(),
                |footer| {
                    footer.child(
                        div()
                            .id("clear")
                            .px_2()
                            .py_1()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .text_xs()
                            .text_color(if self.confirming_clear { danger } else { muted })
                            .hover(move |style| style.text_color(danger))
                            .child(if self.confirming_clear {
                                format!(
                                    "Click again to clear {} items",
                                    text::thousands(self.total)
                                )
                            } else {
                                "Clear history".to_string()
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.clear_history(cx))),
                    )
                },
            )
    }
}

fn plural(count: usize, one: &'static str, many: &'static str) -> &'static str {
    if count == 1 { one } else { many }
}

fn human_bytes(bytes: usize) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.0} KB", bytes as f64 / 1024.),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.),
    }
}
