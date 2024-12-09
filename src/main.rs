// Blackboard App
// Copyright (C) 2024 Seho Lee
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
use eframe::{self, App};
use eframe::egui; // Corrected import for egui
use skia_safe::{Surface, Paint, Path, Font, FontMgr, Typeface, TextBlob, Point};
use skia_safe::gpu::vk::BackendContext; // Correct import for Vulkan BackendContext
use skia_safe::gpu::{DirectContext, Budgeted, SurfaceOrigin};
use ash::{version::InstanceV1_0, version::DeviceV1_0}; // Correct Vulkan imports
use ash::vk;
use std::sync::{Arc, Mutex};
use gstreamer as gst;
use gstreamer::prelude::*;

#[derive(Default)]
struct BlackboardApp {
    drawings: Arc<Mutex<Vec<Path>>>,
    current_path: Path,
    recording: bool,
    rtmp_url: String,
    text_input_mode: bool,
    text_input: String,
    font_size: f32,
    canvas_size: (i32, i32), // Canvas size for 8K resolution
    gst_pipeline: Option<gst::Pipeline>,
    text_orientation: TextOrientation, // Text orientation: Horizontal or Vertical
}

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

impl eframe::App for BlackboardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button(if self.recording { "Stop Recording" } else { "Start Recording" }).clicked() {
                    if self.recording {
                        self.stop_recording();
                    } else {
                        self.start_recording();
                    }
                    self.recording = !self.recording;
                }
                ui.text_edit_singleline(&mut self.rtmp_url);
            });

            if ui.button("Clear Blackboard").clicked() {
                self.drawings.lock().unwrap().clear();
            }

            ui.checkbox(&mut self.text_input_mode, "Text Input Mode");
            if self.text_input_mode {
                ui.horizontal(|ui| {
                    ui.label("Text: ");
                    ui.text_edit_singleline(&mut self.text_input);
                    ui.add(egui::Slider::new(&mut self.font_size, 10.0..=100.0).text("Font Size"));
                });
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.text_orientation, TextOrientation::Horizontal, "Horizontal");
                    ui.radio_value(&mut self.text_orientation, TextOrientation::Vertical, "Vertical");
                });
            }

            ui.label("Draw on the blackboard:");

            let (response, painter) = ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
            if response.hovered() {
                if self.text_input_mode {
                    if response.clicked_by(egui::PointerButton::Primary) {
                        let pos = response.interact_pointer_pos().unwrap();
                        self.draw_text(pos.x, pos.y);
                    }
                } else {
                    if let Some(pointer_pos) = response.interact_pointer_pos() {
                        self.current_path.line_to(pointer_pos.into());
                    }
                    if response.drag_released() {
                        self.drawings.lock().unwrap().push(self.current_path.clone());
                        self.current_path = Path::new();
                    }
                }
            }

            for path in self.drawings.lock().unwrap().iter() {
                painter.path(path.clone(), (2.0, egui::Color32::WHITE));
            }
        });

        ctx.input(|i| {
            if i.key_pressed(egui::Key::C) {
                self.drawings.lock().unwrap().clear();
            }
            if i.key_pressed(egui::Key::T) {
                self.text_input_mode = !self.text_input_mode;
            }
        });
    }
}

impl BlackboardApp {
    fn create_vulkan_context() -> (DirectContext, BackendContext<'static>) {
        // Initialize Vulkan with `ash`
        let entry = ash::Entry::new().unwrap();
        let instance = unsafe {
            entry.create_instance(
                &vk::InstanceCreateInfo {
                    s_type: vk::StructureType::APPLICATION_INFO,
                    p_application_name: std::ffi::CString::new("Blackboard App").unwrap().as_ptr(),
                    p_engine_name: std::ffi::CString::new("No Engine").unwrap().as_ptr(),
                    api_version: vk::make_api_version(1, 0, 0),
                    ..Default::default()
                },
                None,
            )
            .unwrap()
        };

        // Select the first physical device
        let physical_devices = unsafe { instance.enumerate_physical_devices().unwrap() };
        let physical_device = physical_devices[0];

        // Create the Vulkan device
        let device = unsafe {
            instance
                .create_device(
                    physical_device,
                    &vk::DeviceCreateInfo {
                        s_type: vk::StructureType::DEVICE_CREATE_INFO,
                        p_next: std::ptr::null(),
                        ..Default::default()
                    },
                    None,
                )
                .unwrap()
        };
        let queue = unsafe { device.get_device_queue(0, 0) };

        // Set up the Vulkan backend context for Skia
        let backend_context = BackendContext::new_vulkan(&instance, &device, (queue, 0));

        // Create the Vulkan DirectContext
        let context = DirectContext::make_vulkan(&backend_context).unwrap();

        (context, backend_context)
    }

    fn initialize_vulkan_surface(&self) -> Surface {
        let (context, backend_context) = BlackboardApp::create_vulkan_context();
        Surface::new_render_target(
            &context,
            Budgeted::Yes,
            &backend_context,
            SurfaceOrigin::TopLeft,
            None,
        )
        .unwrap()
    }

    fn draw_text(&self, x: f32, y: f32) {
        let font_mgr = FontMgr::default();
        let typeface = Typeface::from_name("Noto Serif KR", skia_safe::FontStyle::default())
            .unwrap_or_else(|| font_mgr.default());
        let font = Font::new(typeface, self.font_size);

        let blob = if self.text_orientation == TextOrientation::Horizontal {
            TextBlob::new(&self.text_input, &font).unwrap()
        } else {
            let mut path = Path::new();
            for (i, ch) in self.text_input.chars().enumerate() {
                let single_char_blob = TextBlob::new(&ch.to_string(), &font).unwrap();
                path.add_text_blob(&single_char_blob, Point::new(x, y + i as f32 * self.font_size));
            }
            TextBlob::new(&self.text_input, &font).unwrap()
        };

        let mut canvas = Surface::new_raster_n32_premul(self.canvas_size).unwrap().canvas();
        let paint = Paint::default().set_color(skia_safe::Color::WHITE);
        canvas.draw_text_blob(&blob, Point::new(x, y), &paint);
    }

    fn start_recording(&mut self) {
        gst::init().unwrap();

        let pipeline_description = format!(
            "appsrc name=src ! videoconvert ! vp8enc ! queue ! mux. audiotestsrc ! audioconvert ! audioresample ! opusenc ! queue ! mux. webmmux streamable=true name=mux ! rtmpsink location={}",
            self.rtmp_url
        );

        let pipeline = gst::parse_launch(&pipeline_description).unwrap();
        self.gst_pipeline = Some(pipeline.dynamic_cast::<gst::Pipeline>().unwrap());
    }

    fn stop_recording(&mut self) {
        if let Some(pipeline) = &self.gst_pipeline {
            pipeline.set_state(gst::State::Null).unwrap();
            self.gst_pipeline = None;
        }
    }
}

fn main() -> Result<(), eframe::Error> {
    let app = BlackboardApp::default();

    eframe::run_native(
        "Blackboard App",
        eframe::NativeOptions {
            initial_window_size: Some(egui::vec2(1920.0, 1080.0)),
            ..Default::default()
        },
        Box::new(|_cc| Box::new(app)),
    )
}
