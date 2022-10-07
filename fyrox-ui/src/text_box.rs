use crate::{
    brush::Brush,
    core::{
        algebra::{Point2, Vector2},
        color::Color,
        math::Rect,
        pool::Handle,
    },
    define_constructor,
    draw::{CommandTexture, Draw, DrawingContext},
    formatted_text::{FormattedText, FormattedTextBuilder, WrapMode},
    message::{CursorIcon, KeyCode, MessageDirection, MouseButton, UiMessage},
    text::TextMessage,
    ttf::SharedFont,
    widget::{Widget, WidgetBuilder, WidgetMessage},
    BuildContext, Control, HorizontalAlignment, UiNode, UserInterface, VerticalAlignment,
    BRUSH_DARKER, BRUSH_TEXT,
};
use copypasta::ClipboardProvider;
use std::{
    any::{Any, TypeId},
    cell::RefCell,
    cmp::Ordering,
    fmt::{Debug, Formatter},
    ops::{Deref, DerefMut},
    rc::Rc,
    sync::mpsc::Sender,
};

/// A message for text box widget.
///
/// # Important notes
///
/// Text box widget also supports [`TextMessage`] and [`WidgetMessage`].
#[derive(Debug, Clone, PartialEq)]
pub enum TextBoxMessage {
    SelectionBrush(Brush),
    CaretBrush(Brush),
    TextCommitMode(TextCommitMode),
    Multiline(bool),
    Editable(bool),
}

impl TextBoxMessage {
    define_constructor!(TextBoxMessage:SelectionBrush => fn selection_brush(Brush), layout: false);
    define_constructor!(TextBoxMessage:CaretBrush => fn caret_brush(Brush), layout: false);
    define_constructor!(TextBoxMessage:TextCommitMode => fn text_commit_mode(TextCommitMode), layout: false);
    define_constructor!(TextBoxMessage:Multiline => fn multiline(bool), layout: false);
    define_constructor!(TextBoxMessage:Editable => fn editable(bool), layout: false);
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum HorizontalDirection {
    Left,
    Right,
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum VerticalDirection {
    Down,
    Up,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct Position {
    // Line index.
    pub line: usize,

    // Offset from beginning of a line.
    pub offset: usize,
}

#[derive(Copy, Clone, PartialOrd, PartialEq, Eq, Ord, Hash, Debug)]
#[repr(u32)]
pub enum TextCommitMode {
    /// Text box will immediately send Text message after any change.
    Immediate = 0,

    /// Text box will send Text message only when it loses focus.
    LostFocus = 1,

    /// Text box will send Text message when it loses focus or if Enter
    /// key was pressed. This is **default** behavior.
    ///
    /// # Notes
    ///
    /// In case of multiline text box hitting Enter key won't commit text!
    LostFocusPlusEnter = 2,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct SelectionRange {
    pub begin: Position,
    pub end: Position,
}

impl SelectionRange {
    #[must_use = "method creates new value which must be used"]
    pub fn normalized(&self) -> SelectionRange {
        match self.begin.line.cmp(&self.end.line) {
            Ordering::Less => *self,
            Ordering::Equal => {
                if self.begin.offset > self.end.offset {
                    SelectionRange {
                        begin: self.end,
                        end: self.begin,
                    }
                } else {
                    *self
                }
            }
            Ordering::Greater => SelectionRange {
                begin: self.end,
                end: self.begin,
            },
        }
    }
}

pub type FilterCallback = dyn FnMut(char) -> bool;

#[derive(Clone)]
pub struct TextBox {
    pub widget: Widget,
    pub caret_position: Position,
    pub caret_visible: bool,
    pub blink_timer: f32,
    pub blink_interval: f32,
    pub formatted_text: RefCell<FormattedText>,
    pub selection_range: Option<SelectionRange>,
    pub selecting: bool,
    pub has_focus: bool,
    pub caret_brush: Brush,
    pub selection_brush: Brush,
    pub filter: Option<Rc<RefCell<FilterCallback>>>,
    pub commit_mode: TextCommitMode,
    pub multiline: bool,
    pub editable: bool,
    pub view_position: Vector2<f32>,
    pub skip_chars: Vec<u32>,
}

impl Debug for TextBox {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("TextBox")
    }
}

crate::define_widget_deref!(TextBox);

impl TextBox {
    fn reset_blink(&mut self) {
        self.caret_visible = true;
        self.blink_timer = 0.0;
    }

    fn move_caret_x(&mut self, mut offset: usize, direction: HorizontalDirection, select: bool) {
        if select {
            if self.selection_range.is_none() {
                self.selection_range = Some(SelectionRange {
                    begin: self.caret_position,
                    end: self.caret_position,
                });
            }
        } else {
            self.selection_range = None;
        }

        self.reset_blink();

        let text = self.formatted_text.borrow();
        let lines = text.get_lines();

        if lines.is_empty() {
            drop(text);
            self.set_caret_position(Default::default());
            return;
        }

        while offset > 0 {
            match direction {
                HorizontalDirection::Left => {
                    if self.caret_position.offset > 0 {
                        self.caret_position.offset -= 1
                    } else if self.caret_position.line > 0 {
                        self.caret_position.line -= 1;
                        self.caret_position.offset = lines[self.caret_position.line].len();
                    } else {
                        self.caret_position.offset = 0;
                        break;
                    }
                }
                HorizontalDirection::Right => {
                    let line = lines.get(self.caret_position.line).unwrap();
                    if self.caret_position.offset < line.len() {
                        self.caret_position.offset += 1;
                    } else if self.caret_position.line < lines.len() - 1 {
                        self.caret_position.line += 1;
                        self.caret_position.offset = 0;
                    } else {
                        self.caret_position.offset = line.len();
                        break;
                    }
                }
            }
            offset -= 1;
        }

        if let Some(selection_range) = self.selection_range.as_mut() {
            if select {
                selection_range.end = self.caret_position;
            }
        }

        drop(text);

        self.ensure_caret_visible();
    }

    fn move_caret_y(&mut self, offset: usize, direction: VerticalDirection, select: bool) {
        if select {
            if self.selection_range.is_none() {
                self.selection_range = Some(SelectionRange {
                    begin: self.caret_position,
                    end: self.caret_position,
                });
            }
        } else {
            self.selection_range = None;
        }

        let text = self.formatted_text.borrow();
        let lines = text.get_lines();

        if lines.is_empty() {
            return;
        }

        let line_count = lines.len();

        match direction {
            VerticalDirection::Down => {
                if self.caret_position.line + offset >= line_count {
                    self.caret_position.line = line_count - 1;
                } else {
                    self.caret_position.line += offset;
                }
            }
            VerticalDirection::Up => {
                if self.caret_position.line > offset {
                    self.caret_position.line -= offset;
                } else {
                    self.caret_position.line = 0;
                }
            }
        }

        if let Some(selection_range) = self.selection_range.as_mut() {
            if select {
                selection_range.end = self.caret_position;
            }
        }

        drop(text);

        self.ensure_caret_visible();
    }

    pub fn position_to_char_index_internal(
        &self,
        position: Position,
        clamp: bool,
    ) -> Option<usize> {
        self.formatted_text
            .borrow()
            .get_lines()
            .get(position.line)
            .map(|line| {
                line.begin
                    + position.offset.min(if clamp {
                        line.len().saturating_sub(1)
                    } else {
                        line.len()
                    })
            })
    }

    /// Maps input [`Position`] to a linear position in character array. Output index can be equal
    /// to length of text, this means that position is at the end of the text. You should check
    /// the index before trying to use it to fetch data from inner array of characters.
    pub fn position_to_char_index_unclamped(&self, position: Position) -> Option<usize> {
        self.position_to_char_index_internal(position, false)
    }

    /// Maps input [`Position`] to a linear position in character array. Output index will always
    /// be valid for fetching, if the method returned `Some(index)`. The index however cannot be
    /// used for text insertion, because it cannot point to a "place after last char".
    pub fn position_to_char_index_clamped(&self, position: Position) -> Option<usize> {
        self.position_to_char_index_internal(position, true)
    }

    pub fn char_index_to_position(&self, i: usize) -> Option<Position> {
        self.formatted_text
            .borrow()
            .get_lines()
            .iter()
            .enumerate()
            .find_map(|(line_index, line)| {
                if (line.begin..=line.end).contains(&i) {
                    Some(Position {
                        line: line_index,
                        offset: i - line.begin,
                    })
                } else {
                    None
                }
            })
    }

    pub fn end_position(&self) -> Position {
        let formatted_text = self.formatted_text.borrow();
        let lines = formatted_text.get_lines();
        lines
            .last()
            .map(|line| Position {
                line: lines.len() - 1,
                offset: line.len(),
            })
            .unwrap_or_default()
    }

    pub fn find_next_word(&self, from: Position) -> Position {
        self.position_to_char_index_unclamped(from)
            .and_then(|i| {
                self.formatted_text
                    .borrow()
                    .get_raw_text()
                    .iter()
                    .enumerate()
                    .skip(i)
                    .skip_while(|(_, c)| {
                        !(c.is_whitespace() || self.skip_chars.contains(&c.char_code))
                    })
                    .find(|(_, c)| !(c.is_whitespace() || self.skip_chars.contains(&c.char_code)))
                    .and_then(|(n, _)| self.char_index_to_position(n))
            })
            .unwrap_or_else(|| self.end_position())
    }

    pub fn find_prev_word(&self, from: Position) -> Position {
        self.position_to_char_index_unclamped(from)
            .and_then(|i| {
                let text = self.formatted_text.borrow();
                let len = text.get_raw_text().len();
                text.get_raw_text()
                    .iter()
                    .enumerate()
                    .rev()
                    .skip(len.saturating_sub(i))
                    .skip_while(|(_, c)| {
                        !(c.is_whitespace() || self.skip_chars.contains(&c.char_code))
                    })
                    .find(|(_, c)| !(c.is_whitespace() || self.skip_chars.contains(&c.char_code)))
                    .and_then(|(n, _)| self.char_index_to_position(n + 1))
            })
            .unwrap_or_default()
    }

    /// Inserts given character at current caret position.
    fn insert_char(&mut self, c: char, ui: &UserInterface) {
        let position = self
            .position_to_char_index_unclamped(self.caret_position)
            .unwrap_or_default();
        self.formatted_text
            .borrow_mut()
            .insert_char(c, position)
            .build();
        self.set_caret_position(
            self.char_index_to_position(position + 1)
                .unwrap_or_default(),
        );
        ui.send_message(TextMessage::text(
            self.handle,
            MessageDirection::ToWidget,
            self.formatted_text.borrow().text(),
        ));
    }

    fn insert_str(&mut self, str: &str, ui: &UserInterface) {
        let position = self
            .position_to_char_index_unclamped(self.caret_position)
            .unwrap_or_default();
        let mut text = self.formatted_text.borrow_mut();
        text.insert_str(str, position);
        text.build();
        drop(text);
        self.set_caret_position(
            self.char_index_to_position(position + str.chars().count())
                .unwrap_or_default(),
        );
        ui.send_message(TextMessage::text(
            self.handle,
            MessageDirection::ToWidget,
            self.formatted_text.borrow().text(),
        ));
    }

    pub fn get_text_len(&self) -> usize {
        self.formatted_text.borrow_mut().get_raw_text().len()
    }

    pub fn caret_local_position(&self) -> Vector2<f32> {
        let text = self.formatted_text.borrow();

        let font = text.get_font();
        let mut caret_pos = Vector2::default();

        let font = font.0.lock();
        if let Some(line) = text.get_lines().get(self.caret_position.line) {
            let text = text.get_raw_text();
            caret_pos += Vector2::new(line.x_offset, line.y_offset);
            for (offset, char_index) in (line.begin..line.end).enumerate() {
                if offset >= self.caret_position.offset {
                    break;
                }
                if let Some(glyph) = text
                    .get(char_index)
                    .and_then(|c| font.glyphs().get(c.glyph_index as usize))
                {
                    caret_pos.x += glyph.advance;
                } else {
                    caret_pos.x += font.height();
                }
            }
        }

        caret_pos
    }

    fn point_to_view_pos(&self, position: Vector2<f32>) -> Vector2<f32> {
        position - self.view_position
    }

    fn rect_to_view_pos(&self, mut rect: Rect<f32>) -> Rect<f32> {
        rect.position -= self.view_position;
        rect
    }

    fn ensure_caret_visible(&mut self) {
        let local_bounds = self.bounding_rect();
        let caret_view_position = self.point_to_view_pos(self.caret_local_position());
        // Move view position to contain the caret + add some spacing.
        let spacing_step = self.formatted_text.borrow().get_font().0.lock().ascender();
        let spacing = spacing_step * 3.0;
        let top_left_corner = local_bounds.left_top_corner();
        let bottom_right_corner = local_bounds.right_bottom_corner();
        if caret_view_position.x > bottom_right_corner.x {
            self.view_position.x += caret_view_position.x - bottom_right_corner.x + spacing;
        }
        if caret_view_position.x < top_left_corner.x {
            self.view_position.x -= top_left_corner.x - caret_view_position.x + spacing;
        }
        if caret_view_position.y > bottom_right_corner.y {
            self.view_position.y += bottom_right_corner.y - caret_view_position.y + spacing;
        }
        if caret_view_position.y < top_left_corner.y {
            self.view_position.y -= top_left_corner.y - caret_view_position.y + spacing;
        }
        self.view_position.x = self.view_position.x.max(0.0);
        self.view_position.y = self.view_position.y.max(0.0);
    }

    fn remove_char(&mut self, direction: HorizontalDirection, ui: &UserInterface) {
        if let Some(position) = self.position_to_char_index_unclamped(self.caret_position) {
            let text_len = self.get_text_len();
            if text_len != 0 {
                let position = match direction {
                    HorizontalDirection::Left => {
                        if position == 0 {
                            return;
                        }
                        position - 1
                    }
                    HorizontalDirection::Right => {
                        if position >= text_len {
                            return;
                        }
                        position
                    }
                };

                let mut text = self.formatted_text.borrow_mut();
                text.remove_at(position);
                text.build();
                drop(text);

                ui.send_message(TextMessage::text(
                    self.handle(),
                    MessageDirection::ToWidget,
                    self.formatted_text.borrow().text(),
                ));

                self.set_caret_position(self.char_index_to_position(position).unwrap_or_default());
            }
        }
    }

    fn remove_range(&mut self, ui: &UserInterface, selection: SelectionRange) {
        let selection = selection.normalized();
        if let Some(begin) = self.position_to_char_index_unclamped(selection.begin) {
            if let Some(end) = self.position_to_char_index_unclamped(selection.end) {
                self.formatted_text.borrow_mut().remove_range(begin..end);
                self.formatted_text.borrow_mut().build();

                ui.send_message(TextMessage::text(
                    self.handle(),
                    MessageDirection::ToWidget,
                    self.formatted_text.borrow().text(),
                ));

                self.set_caret_position(selection.begin);
            }
        }
    }

    pub fn is_valid_position(&self, position: Position) -> bool {
        self.formatted_text
            .borrow()
            .get_lines()
            .get(position.line)
            .map_or(false, |line| position.offset < line.len())
    }

    fn set_caret_position(&mut self, position: Position) {
        self.caret_position = position;
        self.ensure_caret_visible();
        self.reset_blink();
    }

    pub fn screen_pos_to_text_pos(&self, screen_point: Vector2<f32>) -> Option<Position> {
        // Transform given point into local space of the text box - this way calculations can be done
        // as usual, without a need for special math.
        let point_to_check = self
            .visual_transform
            .try_inverse()
            .unwrap_or_default()
            .transform_point(&Point2::from(screen_point))
            .coords;

        if !self.bounding_rect().contains(point_to_check) {
            return None;
        }

        let font = self.formatted_text.borrow().get_font();
        let font = font.0.lock();
        for (line_index, line) in self.formatted_text.borrow().get_lines().iter().enumerate() {
            let line_screen_bounds = Rect::new(
                line.x_offset - self.view_position.x,
                line.y_offset - self.view_position.y,
                line.width,
                font.ascender(),
            );
            if line_screen_bounds.contains(point_to_check) {
                let mut x = line_screen_bounds.x();
                // Check each character in line.
                for (offset, index) in (line.begin..line.end).enumerate() {
                    let character = self.formatted_text.borrow().get_raw_text()[index];
                    let (width, height, advance) =
                        if let Some(glyph) = font.glyphs().get(character.glyph_index as usize) {
                            (
                                glyph.bitmap_width as f32,
                                glyph.bitmap_height as f32,
                                glyph.advance,
                            )
                        } else {
                            // Stub
                            let h = font.height();
                            (h, h, h)
                        };
                    let char_screen_bounds = Rect::new(x, line_screen_bounds.y(), width, height);
                    if char_screen_bounds.contains(point_to_check) {
                        let char_bounds_center_x =
                            char_screen_bounds.x() + char_screen_bounds.w() * 0.5;

                        return Some(Position {
                            line: line_index,
                            offset: if point_to_check.x <= char_bounds_center_x {
                                offset
                            } else {
                                (offset + 1).min(line.len())
                            },
                        });
                    }
                    x += advance;
                }
            }
        }

        // Additionally check each line again, but now check if the cursor is either at left or right side of the cursor.
        // This allows us to set caret at lines by clicking at either ends of it.
        for (line_index, line) in self.formatted_text.borrow().get_lines().iter().enumerate() {
            let line_x_begin = line.x_offset - self.view_position.x;
            let line_x_end = line_x_begin + line.width;
            let line_y_begin = line.y_offset - self.view_position.y;
            let line_y_end = line_y_begin + font.ascender();
            if (line_y_begin..line_y_end).contains(&point_to_check.y) {
                if point_to_check.x < line_x_begin {
                    return Some(Position {
                        line: line_index,
                        offset: 0,
                    });
                } else if point_to_check.x > line_x_end {
                    return Some(Position {
                        line: line_index,
                        offset: line.len(),
                    });
                }
            }
        }

        None
    }

    pub fn text(&self) -> String {
        self.formatted_text.borrow().text()
    }

    pub fn wrap_mode(&self) -> WrapMode {
        self.formatted_text.borrow().wrap_mode()
    }

    pub fn font(&self) -> SharedFont {
        self.formatted_text.borrow().get_font()
    }

    pub fn vertical_alignment(&self) -> VerticalAlignment {
        self.formatted_text.borrow().vertical_alignment()
    }

    pub fn horizontal_alignment(&self) -> HorizontalAlignment {
        self.formatted_text.borrow().horizontal_alignment()
    }

    fn select_word(&mut self, position: Position) {
        if let Some(index) = self.position_to_char_index_clamped(position) {
            let text_ref = self.formatted_text.borrow();
            let text = text_ref.get_raw_text();
            let search_whitespace = !text[index].is_whitespace();

            let mut left_index = index;
            while left_index > 0 {
                let is_whitespace = text[left_index].is_whitespace();
                if search_whitespace && is_whitespace || !search_whitespace && !is_whitespace {
                    left_index += 1;
                    break;
                }
                left_index = left_index.saturating_sub(1);
            }

            let mut right_index = index;
            while right_index < text.len() {
                let is_whitespace = text[right_index].is_whitespace();
                if search_whitespace && is_whitespace || !search_whitespace && !is_whitespace {
                    break;
                }

                right_index += 1;
            }

            drop(text_ref);

            if let (Some(left), Some(right)) = (
                self.char_index_to_position(left_index),
                self.char_index_to_position(right_index),
            ) {
                self.selection_range = Some(SelectionRange {
                    begin: left,
                    end: right,
                });
                self.set_caret_position(right);
            }
        }
    }
}

impl Control for TextBox {
    fn query_component(&self, type_id: TypeId) -> Option<&dyn Any> {
        if type_id == TypeId::of::<Self>() {
            Some(self)
        } else {
            None
        }
    }

    fn measure_override(&self, _: &UserInterface, available_size: Vector2<f32>) -> Vector2<f32> {
        self.formatted_text
            .borrow_mut()
            .set_constraint(available_size)
            .build()
    }

    fn draw(&self, drawing_context: &mut DrawingContext) {
        let bounds = self.widget.bounding_rect();
        drawing_context.push_rect_filled(&bounds, None);
        drawing_context.commit(
            self.clip_bounds(),
            self.widget.background(),
            CommandTexture::None,
            None,
        );

        self.formatted_text
            .borrow_mut()
            .set_constraint(Vector2::new(bounds.w(), bounds.h()))
            .set_brush(self.widget.foreground())
            .build();

        let view_bounds = self.rect_to_view_pos(bounds);
        if let Some(ref selection_range) = self.selection_range.map(|r| r.normalized()) {
            let text = self.formatted_text.borrow();
            let lines = text.get_lines();
            if selection_range.begin.line == selection_range.end.line {
                let line = lines[selection_range.begin.line];
                // Begin line
                let offset =
                    text.get_range_width(line.begin..(line.begin + selection_range.begin.offset));
                let width = text.get_range_width(
                    (line.begin + selection_range.begin.offset)
                        ..(line.begin + selection_range.end.offset),
                );
                let selection_bounds = Rect::new(
                    view_bounds.x() + line.x_offset + offset,
                    view_bounds.y() + line.y_offset,
                    width,
                    line.height,
                );
                drawing_context.push_rect_filled(&selection_bounds, None);
            } else {
                for (i, line) in text.get_lines().iter().enumerate() {
                    if i >= selection_range.begin.line && i <= selection_range.end.line {
                        let selection_bounds = if i == selection_range.begin.line {
                            // Begin line
                            let offset = text.get_range_width(
                                line.begin..(line.begin + selection_range.begin.offset),
                            );
                            let width = text.get_range_width(
                                (line.begin + selection_range.begin.offset)..line.end,
                            );
                            Rect::new(
                                view_bounds.x() + line.x_offset + offset,
                                view_bounds.y() + line.y_offset,
                                width,
                                line.height,
                            )
                        } else if i == selection_range.end.line {
                            // End line
                            let width = text.get_range_width(
                                line.begin..(line.begin + selection_range.end.offset),
                            );
                            Rect::new(
                                view_bounds.x() + line.x_offset,
                                view_bounds.y() + line.y_offset,
                                width,
                                line.height,
                            )
                        } else {
                            // Everything between
                            Rect::new(
                                view_bounds.x() + line.x_offset,
                                view_bounds.y() + line.y_offset,
                                line.width,
                                line.height,
                            )
                        };
                        drawing_context.push_rect_filled(&selection_bounds, None);
                    }
                }
            }
        }
        drawing_context.commit(
            self.clip_bounds(),
            self.selection_brush.clone(),
            CommandTexture::None,
            None,
        );

        let local_position = self.point_to_view_pos(bounds.position);
        drawing_context.draw_text(
            self.clip_bounds(),
            local_position,
            &self.formatted_text.borrow(),
        );

        if self.caret_visible {
            let caret_pos = self.point_to_view_pos(self.caret_local_position());
            let caret_bounds = Rect::new(
                caret_pos.x,
                caret_pos.y,
                2.0,
                self.formatted_text.borrow().get_font().0.lock().height(),
            );
            drawing_context.push_rect_filled(&caret_bounds, None);
            drawing_context.commit(
                self.clip_bounds(),
                self.caret_brush.clone(),
                CommandTexture::None,
                None,
            );
        }
    }

    fn update(&mut self, dt: f32, _sender: &Sender<UiMessage>) {
        if self.has_focus {
            self.blink_timer += dt;
            if self.blink_timer >= self.blink_interval {
                self.blink_timer = 0.0;
                self.caret_visible = !self.caret_visible;
            }
        } else {
            self.caret_visible = false;
        }
    }

    fn handle_routed_message(&mut self, ui: &mut UserInterface, message: &mut UiMessage) {
        self.widget.handle_routed_message(ui, message);

        if message.destination() == self.handle() {
            if let Some(msg) = message.data::<WidgetMessage>() {
                match msg {
                    &WidgetMessage::Text(symbol)
                        if !ui.keyboard_modifiers().control
                            && !ui.keyboard_modifiers().alt
                            && self.editable =>
                    {
                        let insert = if let Some(filter) = self.filter.as_ref() {
                            let filter = &mut *filter.borrow_mut();
                            filter(symbol)
                        } else {
                            true
                        };
                        if insert {
                            if let Some(range) = self.selection_range {
                                self.remove_range(ui, range);
                                self.selection_range = None;
                            }
                            if !symbol.is_control() {
                                self.insert_char(symbol, ui);
                            }
                        }
                    }
                    WidgetMessage::KeyDown(code) => {
                        match code {
                            KeyCode::Up => {
                                self.move_caret_y(
                                    1,
                                    VerticalDirection::Up,
                                    ui.keyboard_modifiers().shift,
                                );
                            }
                            KeyCode::Down => {
                                self.move_caret_y(
                                    1,
                                    VerticalDirection::Down,
                                    ui.keyboard_modifiers().shift,
                                );
                            }
                            KeyCode::Right => {
                                if ui.keyboard_modifiers.control {
                                    let prev_position = self.caret_position;
                                    let next_word_position =
                                        self.find_next_word(self.caret_position);
                                    self.set_caret_position(next_word_position);
                                    self.reset_blink();
                                    if ui.keyboard_modifiers.shift {
                                        if let Some(selection_range) = self.selection_range.as_mut()
                                        {
                                            selection_range.end = next_word_position;
                                        } else {
                                            self.selection_range = Some(SelectionRange {
                                                begin: prev_position,
                                                end: next_word_position,
                                            });
                                        }
                                    } else {
                                        self.selection_range = None;
                                    }
                                } else {
                                    self.move_caret_x(
                                        1,
                                        HorizontalDirection::Right,
                                        ui.keyboard_modifiers().shift,
                                    );
                                }
                            }
                            KeyCode::Left => {
                                if ui.keyboard_modifiers.control {
                                    let prev_position = self.caret_position;
                                    let prev_word_position =
                                        self.find_prev_word(self.caret_position);
                                    self.set_caret_position(prev_word_position);
                                    if ui.keyboard_modifiers.shift {
                                        if let Some(selection_range) = self.selection_range.as_mut()
                                        {
                                            selection_range.end = prev_word_position;
                                        } else {
                                            self.selection_range = Some(SelectionRange {
                                                begin: prev_position,
                                                end: prev_word_position,
                                            });
                                        }
                                    } else {
                                        self.selection_range = None;
                                    }
                                } else {
                                    self.move_caret_x(
                                        1,
                                        HorizontalDirection::Left,
                                        ui.keyboard_modifiers().shift,
                                    );
                                }
                            }
                            KeyCode::Delete if !message.handled() && self.editable => {
                                if let Some(range) = self.selection_range {
                                    self.remove_range(ui, range);
                                    self.selection_range = None;
                                } else {
                                    self.remove_char(HorizontalDirection::Right, ui);
                                }
                            }
                            KeyCode::NumpadEnter | KeyCode::Return if self.editable => {
                                if self.multiline {
                                    self.insert_char('\n', ui);
                                } else if self.commit_mode == TextCommitMode::LostFocusPlusEnter {
                                    ui.send_message(TextMessage::text(
                                        self.handle,
                                        MessageDirection::FromWidget,
                                        self.text(),
                                    ));
                                    self.has_focus = false;
                                }
                            }
                            KeyCode::Backspace if self.editable => {
                                if let Some(range) = self.selection_range {
                                    self.remove_range(ui, range);
                                    self.selection_range = None;
                                } else {
                                    self.remove_char(HorizontalDirection::Left, ui);
                                }
                            }
                            KeyCode::End => {
                                let text = self.formatted_text.borrow();
                                let line = &text.get_lines()[self.caret_position.line];
                                if ui.keyboard_modifiers().control {
                                    let new_position = Position {
                                        line: text.get_lines().len() - 1,
                                        offset: line.end - line.begin,
                                    };
                                    drop(text);
                                    self.set_caret_position(new_position);
                                    self.selection_range = None;
                                } else if ui.keyboard_modifiers().shift {
                                    let prev_position = self.caret_position;
                                    let new_position = Position {
                                        line: self.caret_position.line,
                                        offset: line.end - line.begin,
                                    };
                                    drop(text);
                                    self.set_caret_position(new_position);
                                    self.selection_range = Some(SelectionRange {
                                        begin: prev_position,
                                        end: Position {
                                            line: self.caret_position.line,
                                            offset: self.caret_position.offset,
                                        },
                                    });
                                } else {
                                    let new_position = Position {
                                        line: self.caret_position.line,
                                        offset: line.end - line.begin,
                                    };
                                    drop(text);
                                    self.set_caret_position(new_position);
                                    self.selection_range = None;
                                }
                            }
                            KeyCode::Home => {
                                if ui.keyboard_modifiers().control {
                                    self.set_caret_position(Position { line: 0, offset: 0 });
                                    self.selection_range = None;
                                } else if ui.keyboard_modifiers().shift {
                                    let prev_position = self.caret_position;
                                    self.set_caret_position(Position {
                                        line: self.caret_position.line,
                                        offset: 0,
                                    });
                                    self.selection_range = Some(SelectionRange {
                                        begin: self.caret_position,
                                        end: Position {
                                            line: prev_position.line,
                                            offset: prev_position.offset,
                                        },
                                    });
                                } else {
                                    self.set_caret_position(Position {
                                        line: self.caret_position.line,
                                        offset: 0,
                                    });
                                    self.selection_range = None;
                                }
                            }
                            KeyCode::A if ui.keyboard_modifiers().control => {
                                let text = self.formatted_text.borrow();
                                if let Some(last_line) = &text.get_lines().last() {
                                    self.selection_range = Some(SelectionRange {
                                        begin: Position { line: 0, offset: 0 },
                                        end: Position {
                                            line: text.get_lines().len() - 1,
                                            offset: last_line.end - last_line.begin,
                                        },
                                    });
                                }
                            }
                            KeyCode::C if ui.keyboard_modifiers().control => {
                                if let Some(clipboard) = ui.clipboard_mut() {
                                    if let Some(selection_range) = self.selection_range.as_ref() {
                                        if let (Some(begin), Some(end)) = (
                                            self.position_to_char_index_unclamped(
                                                selection_range.begin,
                                            ),
                                            self.position_to_char_index_unclamped(
                                                selection_range.end,
                                            ),
                                        ) {
                                            let _ = clipboard.set_contents(String::from(
                                                &self.text()[if begin < end {
                                                    begin..end
                                                } else {
                                                    end..begin
                                                }],
                                            ));
                                        }
                                    }
                                }
                            }
                            KeyCode::V if ui.keyboard_modifiers().control => {
                                if let Some(clipboard) = ui.clipboard_mut() {
                                    if let Ok(content) = clipboard.get_contents() {
                                        if let Some(selection_range) = self.selection_range {
                                            self.remove_range(ui, selection_range);
                                            self.selection_range = None;
                                        }

                                        self.insert_str(&content, ui);
                                    }
                                }
                            }
                            _ => (),
                        }

                        // TextBox "eats" all input by default, some of the keys are used for input control while
                        // others are used directly to enter text.
                        message.set_handled(true);
                    }
                    WidgetMessage::Focus => {
                        if message.direction() == MessageDirection::FromWidget {
                            self.reset_blink();
                            self.selection_range = None;
                            self.has_focus = true;
                        }
                    }
                    WidgetMessage::Unfocus => {
                        if message.direction() == MessageDirection::FromWidget {
                            self.selection_range = None;
                            self.has_focus = false;

                            if self.commit_mode == TextCommitMode::LostFocus
                                || self.commit_mode == TextCommitMode::LostFocusPlusEnter
                            {
                                ui.send_message(TextMessage::text(
                                    self.handle,
                                    MessageDirection::FromWidget,
                                    self.text(),
                                ));
                            }
                        }
                    }
                    WidgetMessage::MouseDown { pos, button } => {
                        if *button == MouseButton::Left {
                            self.selection_range = None;
                            self.selecting = true;
                            self.has_focus = true;

                            if let Some(position) = self.screen_pos_to_text_pos(*pos) {
                                self.set_caret_position(position);
                            }

                            ui.capture_mouse(self.handle());
                        }
                    }
                    WidgetMessage::DoubleClick {
                        button: MouseButton::Left,
                    } => {
                        if let Some(position) = self.screen_pos_to_text_pos(ui.cursor_position) {
                            self.select_word(position);
                        }
                    }
                    WidgetMessage::MouseMove { pos, .. } => {
                        if self.selecting {
                            if let Some(position) = self.screen_pos_to_text_pos(*pos) {
                                if let Some(ref mut selection_range) = self.selection_range {
                                    selection_range.end = position;
                                    self.set_caret_position(position);
                                } else if position != self.caret_position {
                                    self.selection_range = Some(SelectionRange {
                                        begin: self.caret_position,
                                        end: position,
                                    })
                                }
                            }
                        }
                    }
                    WidgetMessage::MouseUp { .. } => {
                        self.selecting = false;

                        ui.release_mouse_capture();
                    }
                    _ => {}
                }
            } else if let Some(msg) = message.data::<TextMessage>() {
                if message.direction() == MessageDirection::ToWidget {
                    let mut text = self.formatted_text.borrow_mut();

                    match msg {
                        TextMessage::Text(new_text) => {
                            let mut equals = false;
                            for (&old, new) in text.get_raw_text().iter().zip(new_text.chars()) {
                                if old.char_code != new as u32 {
                                    equals = false;
                                    break;
                                }
                            }
                            if !equals {
                                text.set_text(new_text);
                                drop(text);
                                self.invalidate_layout();
                                self.formatted_text.borrow_mut().build();

                                if self.commit_mode == TextCommitMode::Immediate {
                                    ui.send_message(message.reverse());
                                }
                            }
                        }
                        TextMessage::Wrap(wrap_mode) => {
                            if text.wrap_mode() != *wrap_mode {
                                text.set_wrap(*wrap_mode);
                                drop(text);
                                self.invalidate_layout();
                                ui.send_message(message.reverse());
                            }
                        }
                        TextMessage::Font(font) => {
                            if &text.get_font() != font {
                                text.set_font(font.clone());
                                drop(text);
                                self.invalidate_layout();
                                ui.send_message(message.reverse());
                            }
                        }
                        TextMessage::VerticalAlignment(alignment) => {
                            if &text.vertical_alignment() != alignment {
                                text.set_vertical_alignment(*alignment);
                                drop(text);
                                self.invalidate_layout();
                                ui.send_message(message.reverse());
                            }
                        }
                        TextMessage::HorizontalAlignment(alignment) => {
                            if &text.horizontal_alignment() != alignment {
                                text.set_horizontal_alignment(*alignment);
                                drop(text);
                                self.invalidate_layout();
                                ui.send_message(message.reverse());
                            }
                        }
                        &TextMessage::Shadow(shadow) => {
                            if text.shadow != shadow {
                                text.set_shadow(shadow);
                                drop(text);
                                self.invalidate_layout();
                                ui.send_message(message.reverse());
                            }
                        }
                        TextMessage::ShadowBrush(brush) => {
                            if &text.shadow_brush != brush {
                                text.set_shadow_brush(brush.clone());
                                drop(text);
                                self.invalidate_layout();
                                ui.send_message(message.reverse());
                            }
                        }
                        &TextMessage::ShadowDilation(dilation) => {
                            if text.shadow_dilation != dilation {
                                text.set_shadow_dilation(dilation);
                                drop(text);
                                self.invalidate_layout();
                                ui.send_message(message.reverse());
                            }
                        }
                        &TextMessage::ShadowOffset(offset) => {
                            if text.shadow_offset != offset {
                                text.set_shadow_offset(offset);
                                drop(text);
                                self.invalidate_layout();
                                ui.send_message(message.reverse());
                            }
                        }
                    }
                }
            } else if let Some(msg) = message.data::<TextBoxMessage>() {
                if message.direction() == MessageDirection::ToWidget {
                    match msg {
                        TextBoxMessage::SelectionBrush(brush) => {
                            if &self.selection_brush != brush {
                                self.selection_brush = brush.clone();
                                ui.send_message(message.reverse());
                            }
                        }
                        TextBoxMessage::CaretBrush(brush) => {
                            if &self.caret_brush != brush {
                                self.caret_brush = brush.clone();
                                ui.send_message(message.reverse());
                            }
                        }
                        TextBoxMessage::TextCommitMode(mode) => {
                            if &self.commit_mode != mode {
                                self.commit_mode = *mode;
                                ui.send_message(message.reverse());
                            }
                        }
                        TextBoxMessage::Multiline(multiline) => {
                            if &self.multiline != multiline {
                                self.multiline = *multiline;
                                ui.send_message(message.reverse());
                            }
                        }
                        TextBoxMessage::Editable(editable) => {
                            if &self.editable != editable {
                                self.editable = *editable;
                                ui.send_message(message.reverse());
                            }
                        }
                    }
                }
            }
        }
    }
}

pub struct TextBoxBuilder {
    widget_builder: WidgetBuilder,
    font: Option<SharedFont>,
    text: String,
    caret_brush: Brush,
    selection_brush: Brush,
    filter: Option<Rc<RefCell<FilterCallback>>>,
    vertical_alignment: VerticalAlignment,
    horizontal_alignment: HorizontalAlignment,
    wrap: WrapMode,
    commit_mode: TextCommitMode,
    multiline: bool,
    editable: bool,
    mask_char: Option<char>,
    shadow: bool,
    shadow_brush: Brush,
    shadow_dilation: f32,
    shadow_offset: Vector2<f32>,
    skip_chars: Vec<u32>,
}

impl TextBoxBuilder {
    pub fn new(widget_builder: WidgetBuilder) -> Self {
        Self {
            widget_builder,
            font: None,
            text: "".to_owned(),
            caret_brush: Brush::Solid(Color::WHITE),
            selection_brush: Brush::Solid(Color::opaque(80, 118, 178)),
            filter: None,
            vertical_alignment: VerticalAlignment::Top,
            horizontal_alignment: HorizontalAlignment::Left,
            wrap: WrapMode::NoWrap,
            commit_mode: TextCommitMode::LostFocusPlusEnter,
            multiline: false,
            editable: true,
            mask_char: None,
            shadow: false,
            shadow_brush: Brush::Solid(Color::BLACK),
            shadow_dilation: 1.0,
            shadow_offset: Vector2::new(1.0, 1.0),
            skip_chars: Default::default(),
        }
    }

    pub fn with_font(mut self, font: SharedFont) -> Self {
        self.font = Some(font);
        self
    }

    pub fn with_text<P: AsRef<str>>(mut self, text: P) -> Self {
        self.text = text.as_ref().to_owned();
        self
    }

    pub fn with_caret_brush(mut self, brush: Brush) -> Self {
        self.caret_brush = brush;
        self
    }

    pub fn with_selection_brush(mut self, brush: Brush) -> Self {
        self.selection_brush = brush;
        self
    }

    pub fn with_filter(mut self, filter: Rc<RefCell<FilterCallback>>) -> Self {
        self.filter = Some(filter);
        self
    }

    pub fn with_vertical_text_alignment(mut self, alignment: VerticalAlignment) -> Self {
        self.vertical_alignment = alignment;
        self
    }

    pub fn with_horizontal_text_alignment(mut self, alignment: HorizontalAlignment) -> Self {
        self.horizontal_alignment = alignment;
        self
    }

    pub fn with_wrap(mut self, wrap: WrapMode) -> Self {
        self.wrap = wrap;
        self
    }

    pub fn with_text_commit_mode(mut self, mode: TextCommitMode) -> Self {
        self.commit_mode = mode;
        self
    }

    pub fn with_multiline(mut self, multiline: bool) -> Self {
        self.multiline = multiline;
        self
    }

    pub fn with_editable(mut self, editable: bool) -> Self {
        self.editable = editable;
        self
    }

    pub fn with_mask_char(mut self, mask_char: Option<char>) -> Self {
        self.mask_char = mask_char;
        self
    }

    /// Whether the shadow enabled or not.
    pub fn with_shadow(mut self, shadow: bool) -> Self {
        self.shadow = shadow;
        self
    }

    /// Sets desired shadow brush. It will be used to render the shadow.
    pub fn with_shadow_brush(mut self, brush: Brush) -> Self {
        self.shadow_brush = brush;
        self
    }

    /// Sets desired shadow dilation in units. Keep in mind that the dilation is absolute,
    /// not percentage-based.
    pub fn with_shadow_dilation(mut self, thickness: f32) -> Self {
        self.shadow_dilation = thickness;
        self
    }

    /// Sets desired shadow offset in units.
    pub fn with_shadow_offset(mut self, offset: Vector2<f32>) -> Self {
        self.shadow_offset = offset;
        self
    }

    /// Sets desired set of characters that will be treated like whitespace during Ctrl+Arrow navigation
    /// (Ctrl+Left Arrow and Ctrl+Right Arrow). This could be useful to treat underscores like whitespaces,
    /// which in its turn could be useful for in-game consoles where commands usually separated using
    /// underscores (`like_this_one`).
    pub fn with_skip_chars(mut self, chars: Vec<char>) -> Self {
        self.skip_chars = chars.into_iter().map(|c| c as u32).collect();
        self
    }

    pub fn build(mut self, ctx: &mut BuildContext) -> Handle<UiNode> {
        if self.widget_builder.foreground.is_none() {
            self.widget_builder.foreground = Some(BRUSH_TEXT);
        }
        if self.widget_builder.background.is_none() {
            self.widget_builder.background = Some(BRUSH_DARKER);
        }
        if self.widget_builder.cursor.is_none() {
            self.widget_builder.cursor = Some(CursorIcon::Text);
        }

        let text_box = TextBox {
            widget: self.widget_builder.build(),
            caret_position: Position::default(),
            caret_visible: false,
            blink_timer: 0.0,
            blink_interval: 0.5,
            formatted_text: RefCell::new(
                FormattedTextBuilder::new(self.font.unwrap_or_else(|| ctx.default_font()))
                    .with_text(self.text)
                    .with_horizontal_alignment(self.horizontal_alignment)
                    .with_vertical_alignment(self.vertical_alignment)
                    .with_wrap(self.wrap)
                    .with_mask_char(self.mask_char)
                    .with_shadow(self.shadow)
                    .with_shadow_brush(self.shadow_brush)
                    .with_shadow_dilation(self.shadow_dilation)
                    .with_shadow_offset(self.shadow_offset)
                    .build(),
            ),
            selection_range: None,
            selecting: false,
            selection_brush: self.selection_brush,
            caret_brush: self.caret_brush,
            has_focus: false,
            filter: self.filter,
            commit_mode: self.commit_mode,
            multiline: self.multiline,
            editable: self.editable,
            view_position: Default::default(),
            skip_chars: self.skip_chars,
        };

        ctx.add_node(UiNode::new(text_box))
    }
}
