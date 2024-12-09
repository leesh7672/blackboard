// Blackboard App
// Copyright (C) 2024
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

// Blackboard App
// This version uses `fc-list` to discover all system fonts and load them into egui.

use eframe::egui;
use egui::epaint::{PathShape, Shape};
use gstreamer as gst;
use gstreamer::prelude::*;
use egui::FontId;
use std::process::Command;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Copy, Clone, PartialEq)]
enum TextOrientation {
    Horizontal,
    Vertical,
}

impl Default for TextOrientation {
    fn default() -> Self {
        TextOrientation::Horizontal
    }
}

struct BlackboardApp {
    drawings: std::sync::Arc<std::sync::Mutex<Vec<Vec<egui::Pos2>>>>,
    current_line: Vec<egui::Pos2>,
    recording_rtmp: bool,
    recording_file: bool,
    rtmp_url: String,
    output_file_path: String,
    text_input_mode: bool,
    text_input: String,
    font_size: f32,
    gst_pipeline: Option<gst::Pipeline>,
    text_orientation: TextOrientation,
    eraser_mode: bool,
    placed_texts: Vec<(egui::Pos2, String, f32, TextOrientation, String)>,
    available_fonts: Vec<String>,
    selected_font: String,
}

impl Default for BlackboardApp {
    fn default() -> Self {
        BlackboardApp {
            drawings: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            current_line: Vec::new(),
            recording_rtmp: false,
            recording_file: false,
            rtmp_url: "rtmp://example.com/live/streamkey".to_string(),
            output_file_path: "output.webm".to_string(),
            text_input_mode: false,
            text_input: String::new(),
            font_size: 40.0,
            gst_pipeline: None,
            text_orientation: TextOrientation::default(),
            eraser_mode: false,
            placed_texts: Vec::new(),
            available_fonts: Vec::new(),
            selected_font: String::new(),
        }
    }
}

impl eframe::App for BlackboardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Top toolbar
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // RTMP Recording
                let rtmp_label = if self.recording_rtmp { "Stop RTMP" } else { "Start RTMP" };
                if ui.button(rtmp_label).clicked() {
                    if self.recording_rtmp {
                        if let Err(e) = self.stop_recording() {
                            eprintln!("Error stopping RTMP recording: {}", e);
                        }
                        self.recording_rtmp = false;
                    } else {
                        if let Err(e) = self.start_recording_rtmp() {
                            eprintln!("Error starting RTMP recording: {}", e);
                        }
                        self.recording_rtmp = true;
                    }
                }
                ui.text_edit_singleline(&mut self.rtmp_url);

                // File Recording
                let file_label = if self.recording_file { "Stop File" } else { "Start File" };
                if ui.button(file_label).clicked() {
                    if self.recording_file {
                        if let Err(e) = self.stop_recording() {
                            eprintln!("Error stopping file recording: {}", e);
                        }
                        self.recording_file = false;
                    } else {
                        if let Err(e) = self.start_recording_file() {
                            eprintln!("Error starting file recording: {}", e);
                        }
                        self.recording_file = true;
                    }
                }
                ui.text_edit_singleline(&mut self.output_file_path);

                // Clear board
                if ui.button("Clear").clicked() {
                    self.drawings.lock().unwrap().clear();
                    self.placed_texts.clear();
                }

                // Text input mode
                {
                    let text_mode_label = if self.text_input_mode { "Text: ON" } else { "Text: OFF" };
                    if ui.button(text_mode_label).clicked() {
                        self.text_input_mode = !self.text_input_mode;
                        if self.text_input_mode {
                            // Disable eraser if text mode is on
                            self.eraser_mode = false;
                        }
                    }
                }

                if self.text_input_mode {
                    ui.label("Text:");
                    ui.text_edit_singleline(&mut self.text_input);
                    ui.add(egui::Slider::new(&mut self.font_size, 10.0..=100.0).text("Font Size"));
                    ui.radio_value(&mut self.text_orientation, TextOrientation::Horizontal, "Horizontal");
                    ui.radio_value(&mut self.text_orientation, TextOrientation::Vertical, "Vertical");

                    ui.label("Font:");
                    egui::ComboBox::from_id_source("font_selector")
                        .selected_text(&self.selected_font)
                        .show_ui(ui, |ui| {
                            for font in &self.available_fonts {
                                ui.selectable_value(&mut self.selected_font, font.clone(), font);
                            }
                        });
                }

                // Eraser mode
                {
                    let eraser_label = if self.eraser_mode { "Eraser: ON" } else { "Eraser: OFF" };
                    if ui.button(eraser_label).clicked() {
                        self.eraser_mode = !self.eraser_mode;
                        if self.eraser_mode {
                            self.text_input_mode = false;
                        }
                    }
                }
            });
        });

        // Main drawing area
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("Draw on the blackboard. Use Eraser: ON and click/drag near lines or texts to remove them.");
            let (response, painter) = ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());

            // Place text
            if self.text_input_mode && response.clicked() {
                if let Some(pointer_pos) = response.interact_pointer_pos() {
                    if !self.text_input.is_empty() && !self.selected_font.is_empty() {
                        self.placed_texts.push((
                            pointer_pos,
                            self.text_input.clone(),
                            self.font_size,
                            self.text_orientation,
                            self.selected_font.clone(),
                        ));
                    }
                }
            }

            // Erase
            if self.eraser_mode && response.clicked() {
                if let Some(pointer_pos) = response.interact_pointer_pos() {
                    self.erase_near(pointer_pos);
                }
            }

            if response.drag_started() && !self.eraser_mode && !self.text_input_mode {
                if let Some(pointer_pos) = response.interact_pointer_pos() {
                    self.current_line.clear();
                    self.current_line.push(pointer_pos);
                }
            }

            if response.dragged() {
                if let Some(pointer_pos) = response.interact_pointer_pos() {
                    if self.eraser_mode {
                        self.erase_near(pointer_pos);
                    } else if !self.text_input_mode {
                        self.current_line.push(pointer_pos);
                    }
                }
            }

            if response.drag_released() && !self.eraser_mode && !self.text_input_mode {
                if !self.current_line.is_empty() {
                    self.drawings.lock().unwrap().push(self.current_line.clone());
                    self.current_line.clear();
                }
            }

            // Render lines
            for line in self.drawings.lock().unwrap().iter() {
                painter.add(Shape::Path(PathShape {
                    points: line.clone(),
                    closed: false,
                    fill: egui::Color32::TRANSPARENT,
                    stroke: egui::Stroke::new(2.0, egui::Color32::WHITE),
                }));
            }

            // Render current line
            if !self.eraser_mode && !self.text_input_mode && !self.current_line.is_empty() {
                painter.add(Shape::Path(PathShape {
                    points: self.current_line.clone(),
                    closed: false,
                    fill: egui::Color32::TRANSPARENT,
                    stroke: egui::Stroke::new(2.0, egui::Color32::WHITE),
                }));
            }

            // Render texts
            for (pos, text, size, orientation, font_name) in &self.placed_texts {
                let displayed_text = if *orientation == TextOrientation::Horizontal {
                    text.clone()
                } else {
                    let mut vtext = String::new();
                    for (i, ch) in text.chars().enumerate() {
                        if i > 0 {
                            vtext.push('\n');
                        }
                        vtext.push(ch);
                    }
                    vtext
                };

                let font_id = FontId::new(*size, egui::FontFamily::Name(Arc::from(font_name.as_str())));
                painter.text(
                    *pos,
                    egui::Align2::LEFT_TOP,
                    &displayed_text,
                    font_id,
                    egui::Color32::WHITE,
                );
            }
        });

        if self.recording_rtmp || self.recording_file {
            ctx.request_repaint();
        }
    }
}

impl BlackboardApp {
    fn start_recording_rtmp(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        gst::init()?;
        let pipeline_description = format!(
            "videotestsrc ! videoconvert ! vp8enc ! queue ! mux. \
             pulsesrc ! audioconvert ! audioresample ! opusenc ! queue ! mux. \
             webmmux streamable=true name=mux ! rtmpsink location={}",
            self.rtmp_url
        );
        let pipeline = gst::parse_launch(&pipeline_description)?
            .dynamic_cast::<gst::Pipeline>()
            .map_err(|_| "Failed to cast to Pipeline")?;
        pipeline.set_state(gst::State::Playing)?;
        self.gst_pipeline = Some(pipeline);
        Ok(())
    }

    fn start_recording_file(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        gst::init()?;
        let pipeline_description = format!(
            "videotestsrc ! videoconvert ! vp8enc ! queue ! mux. \
             pulsesrc ! audioconvert ! audioresample ! opusenc ! queue ! mux. \
             webmmux streamable=true name=mux ! filesink location={} sync=false",
            self.output_file_path
        );
        let pipeline = gst::parse_launch(&pipeline_description)?
            .dynamic_cast::<gst::Pipeline>()
            .map_err(|_| "Failed to cast to Pipeline")?;
        pipeline.set_state(gst::State::Playing)?;
        self.gst_pipeline = Some(pipeline);
        Ok(())
    }

    fn stop_recording(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(pipeline) = &self.gst_pipeline {
            pipeline.set_state(gst::State::Null)?;
            self.gst_pipeline = None;
        }
        Ok(())
    }

    fn erase_near(&mut self, pointer_pos: egui::Pos2) {
        let erase_radius = 20.0;
        {
            let mut drawings = self.drawings.lock().unwrap();
            drawings.retain(|line| {
                !line.iter().any(|&pt| {
                    let dx = pt.x - pointer_pos.x;
                    let dy = pt.y - pointer_pos.y;
                    (dx * dx + dy * dy).sqrt() < erase_radius
                })
            });
        }

        self.placed_texts.retain(|(pos, _text, _size, _orient, _font)| {
            let dx = pos.x - pointer_pos.x;
            let dy = pos.y - pointer_pos.y;
            (dx * dx + dy * dy).sqrt() >= erase_radius
        });
    }
}

/// Runs `fc-list` and returns a vector of (family, path).
fn list_all_fonts() -> Vec<(String, String)> {
    let output = Command::new("fc-list")
        .arg(":")
        .output()
        .expect("Failed to execute fc-list");
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut family_to_path = HashMap::new();

    for line in stdout.lines() {
        // Expected format: "path: family:style=..."
        // Example:
        // "/usr/local/share/fonts/NotoSansKR-Regular.ttf: Noto Sans KR:style=Regular"
        if let Some((path_part, rest)) = line.split_once(':') {
            let parts: Vec<_> = rest.trim().split(':').collect();
            if !parts.is_empty() {
                let family = parts[0].trim().to_string();
                let path = path_part.trim().to_string();
                if !family.is_empty() && !path.is_empty() && !family_to_path.contains_key(&family) {
                    family_to_path.insert(family, path);
                }
            }
        }
    }

    family_to_path.into_iter().collect()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let native_options = eframe::NativeOptions::default();

    // Build FontDefinitions first
    let mut defs = egui::FontDefinitions::default();
    let fonts_list = list_all_fonts(); // from fc-list

    for (family, path) in fonts_list {
        let data = std::fs::read(&path).unwrap_or_else(|_| {
            eprintln!("Could not read font file: {}", path);
            Vec::new()
        });
        if !data.is_empty() {
            let key = family.replace(' ', "");
            defs.font_data.insert(key.clone(), egui::FontData::from_owned(data));
            defs.families.insert(
                egui::FontFamily::Name(Arc::from(family.as_str())),
                vec![key],
            );
        }
    }

    // Sort family names now (just the keys)
    let mut all_families: Vec<String> = defs
        .families
        .keys()
        .map(|fam| match fam {
            egui::FontFamily::Name(name) => name.to_string(),
            egui::FontFamily::Monospace => "Monospace".into(),
            egui::FontFamily::Proportional => "Proportional".into(),
        })
        .collect();
    all_families.sort();
    eframe::run_native(
        "Blackboard App - All System Fonts",
        native_options,
        Box::new(move |cc| {
            cc.egui_ctx.set_fonts(defs.clone());
    
            let mut app = BlackboardApp::default();
            app.available_fonts = all_families;
            if !app.available_fonts.is_empty() {
                app.selected_font = app.available_fonts[0].clone();
            }
    
            Box::new(app)
        }),
    )?;    

    Ok(())
}
