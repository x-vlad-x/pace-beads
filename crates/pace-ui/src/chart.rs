// SPDX-License-Identifier: GPL-3.0-only
//! Bar chart drawn with Cairo.

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{accessible, cairo, gdk};
use pace_core::stats::{Bucket, Resolution};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default)]
struct Series {
    resolution: Option<Resolution>,
    bars: Vec<Bucket>,
    goal: Option<u64>,
}

#[derive(Clone)]
pub struct Chart {
    pub widget: gtk::DrawingArea,
    series: Rc<RefCell<Series>>,
}

const LABEL_SPACE: f64 = 18.0;
const TOP_SPACE: f64 = 6.0;
const MIN_LABEL_WIDTH: f64 = 22.0;

fn label(resolution: Resolution, bar: &Bucket) -> String {
    match resolution {
        Resolution::Day => bar.first.format("%-d").to_string(),
        Resolution::Month => bar.first.format("%m").to_string(),
        Resolution::Year => bar.first.format("%Y").to_string(),
    }
}

fn set_source(context: &cairo::Context, color: gdk::RGBA) {
    context.set_source_rgba(
        f64::from(color.red()),
        f64::from(color.green()),
        f64::from(color.blue()),
        f64::from(color.alpha()),
    );
}

fn draw(widget: &gtk::DrawingArea, context: &cairo::Context, width: i32, height: i32, series: &Series) {
    let Some(resolution) = series.resolution else {
        return;
    };
    if series.bars.is_empty() {
        return;
    }
    let width = f64::from(width);
    let plot_height = f64::from(height) - LABEL_SPACE - TOP_SPACE;
    let largest = series.bars.iter().map(|bar| bar.steps).max().unwrap_or(0);
    let scale = largest.max(series.goal.unwrap_or(0)).max(1) as f64;
    let slot = width / series.bars.len() as f64;
    let bar_width = (slot * 0.7).max(1.0);
    let foreground = widget.color();
    let accent = adw::StyleManager::default().accent_color_rgba();
    let step = (MIN_LABEL_WIDTH / slot).ceil().max(1.0) as usize;

    context.set_font_size(11.0);
    for (index, bar) in series.bars.iter().enumerate() {
        let x = index as f64 * slot + (slot - bar_width) / 2.0;
        let height = bar.steps as f64 / scale * plot_height;
        set_source(context, accent);
        context.rectangle(x, TOP_SPACE + plot_height - height, bar_width, height);
        let _ = context.fill();

        let from_end = series.bars.len() - 1 - index;
        if from_end % step == 0 {
            let text = label(resolution, bar);
            if let Ok(extents) = context.text_extents(&text) {
                set_source(context, foreground);
                let center = index as f64 * slot + slot / 2.0 - extents.width() / 2.0;
                context.move_to(center.clamp(0.0, (width - extents.width()).max(0.0)), TOP_SPACE + plot_height + 14.0);
                let _ = context.show_text(&text);
            }
        }
    }

    if let Some(goal) = series.goal {
        let y = TOP_SPACE + plot_height - goal as f64 / scale * plot_height;
        let mut line = foreground;
        line.set_alpha(0.6);
        set_source(context, line);
        context.set_dash(&[4.0, 4.0], 0.0);
        context.set_line_width(1.0);
        context.move_to(0.0, y);
        context.line_to(width, y);
        let _ = context.stroke();
    }
}

impl Chart {
    pub fn new() -> Self {
        let widget = gtk::DrawingArea::builder()
            .content_height(170)
            .hexpand(true)
            .accessible_role(gtk::AccessibleRole::Img)
            .build();
        let series = Rc::new(RefCell::new(Series::default()));
        let shared = series.clone();
        widget.set_draw_func(move |area, context, width, height| {
            draw(area, context, width, height, &shared.borrow());
        });
        adw::StyleManager::default().connect_accent_color_notify({
            let widget = widget.clone();
            move |_| widget.queue_draw()
        });
        Self { widget, series }
    }

    pub fn set_data(&self, resolution: Resolution, bars: Vec<Bucket>, goal: Option<u64>) {
        let total: u64 = bars.iter().map(|bar| bar.steps).sum();
        let description = crate::format::fill(
            &gettext("Bar chart of {}, {} in total"),
            &[&crate::format::period_count(bars.len() as u64), &crate::format::step_count(total)],
        );
        self.widget.update_property(&[accessible::Property::Label(&description)]);
        *self.series.borrow_mut() = Series { resolution: Some(resolution), bars, goal };
        self.widget.queue_draw();
    }
}
