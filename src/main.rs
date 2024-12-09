// Blackboard App
// Copyright (C) 2024 [Your Name]
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

use eframe::{egui, epi};
use winit::platform::unix::EventLoopExtUnix;
use winit::{dpi::PhysicalPosition, event::*, event_loop::EventLoop, window::WindowBuilder};
use skia_safe::{gpu, vulkan, Surface, Canvas, Paint, Path, Font, FontMgr, Typeface, TextBlob, Point, TextAlign};
use skia_safe::gpu::{BackendRenderTarget, DirectContext};
use std::sync::{Arc, Mutex};
use gstreamer as gst;
use gstreamer_video as gst_video;

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

impl epi::App for BlackboardApp {
    fn name(&self) -> &str {
        "Blackboard App"
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &epi::Frame) {
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
    fn create_vulkan_context() -> (DirectContext, BackendRenderTarget) {
        let instance = vulkan::Instance::new(None, &[], &[]).unwrap();
        let adapter = instance.get_physical_device();
        let queue_family_index = adapter.queue_family_indices()[0];

        let backend_context = vulkan::BackendContext {
            instance: instance.clone(),
            physical_device: adapter.physical_device(),
            device: adapter.device(),
            queue: adapter.queue(),
            queue_family_index,
        };

        let context = DirectContext::new_vulkan(&backend_context, None).unwrap();

        let backend_render_target = BackendRenderTarget::new_vulkan(
            (7680, 4320),
            1,
            &backend_context,
        );

        (context, backend_render_target)
    }

    fn initialize_vulkan_surface(&self) -> Surface {
        let (context, backend_render_target) = BlackboardApp::create_vulkan_context();
        Surface::new_render_target(
            &context,
            backend_render_target,
            gpu::SurfaceOrigin::TopLeft,
            None,
        )
        .expect("Failed to create Vulkan surface")
    }

    fn draw_text(&self, x: f32, y: f32) {
        let font_mgr = FontMgr::default();
        let typeface = Typeface::new("Noto Serif KR", skia_safe::FontStyle::default()).unwrap_or_else(|| font_mgr.legacy_default());
        let font = Font::new(typeface, self.font_size);

        let blob = if self.text_orientation == TextOrientation::Horizontal {
            TextBlob::new(&self.text_input, &font).expect("Failed to create text blob")
        } else {
            let mut path = Path::new();
            for (i, ch) in self.text_input.chars().enumerate() {
                let single_char_blob = TextBlob::new(&ch.to_string(), &font).expect("Failed to create text blob");
                path.add_text_blob(&single_char_blob, Point::new(x, y + i as f32 * self.font_size));
            }
            TextBlob::from_path(&path, &font).expect("Failed to create vertical text blob")
        };

        let mut canvas = Surface::new_raster_n32_premul(self.canvas_size).expect("Failed to create Skia surface").canvas();

        let paint = Paint::default().set_color(skia_safe::Color::WHITE);
        canvas.draw_text_blob(&blob, Point::new(x, y), &paint);
    }

    fn start_recording(&mut self) {
        gst::init().expect("Failed to initialize GStreamer");

        let pipeline_description = format!(
            "appsrc name=src ! videoconvert ! vp8enc ! queue ! mux. audiotestsrc ! audioconvert ! audioresample ! opusenc ! queue ! mux. webmmux streamable=true name=mux ! rtmpsink location={}",
            self.rtmp_url
        );

        let pipeline = gst::parse_launch(&pipeline_description).expect("Failed to create pipeline");
        let appsrc = pipeline
            .dynamic_cast::<gst::Pipeline>()
            .unwrap()
            .by_name("src")
            .unwrap()
            .dynamic_cast::<gst::AppSrc>()
            .unwrap();

        appsrc.set_caps(Some(&gst::Caps::builder("video/x-raw")
            .field("format", &"RGB")
            .field("width", &(self.canvas_size.0 as i32))
            .field("height", &(self.canvas_size.1 as i32))
            .field("framerate", &gst::Fraction::new(30, 1))
            .build()));

        pipeline.set_state(gst::State::Playing).expect("Unable to set the pipeline to Playing state");
        self.gst_pipeline = Some(pipeline);
    }

    fn stop_recording(&mut self) {
        if let Some(pipeline) = &self.gst_pipeline {
            pipeline.set_state(gst::State::Null).expect("Unable to set the pipeline to Null state");
            self.gst_pipeline = None;
        }
    }
}

fn main() -> Result<(), eframe::Error> {
    let event_loop = EventLoop::new_any_thread();
    let window = WindowBuilder::new()
        .with_title("Blackboard App with Vulkan")
        .build(&event_loop)
        .unwrap();

    let app = Arc::new(Mutex::new(BlackboardApp {
        canvas_size: (7680, 4320), // 8K resolution
        ..Default::default()
    }));

    let vulkan_surface = app.lock().unwrap().initialize_vulkan_surface();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => *control_flow = ControlFlow::Exit,
                WindowEvent::MouseInput { state, button, .. } => {
                    let mut app = app.lock().unwrap();
                    if button == MouseButton::Left && state == ElementState::Pressed {
                        app.current_path = Path::new();
                    }
                }
                _ => {}
            },
            Event::RedrawRequested(_) => {
                let mut canvas = vulkan_surface.canvas();
                canvas.clear(skia_safe::Color::BLACK);

                let mut app = app.lock().unwrap();
                for path in app.drawings.lock().unwrap().iter() {
                    let paint = Paint::default().set_color(skia_safe::Color::WHITE);
                    canvas.draw_path(path, &paint);
                }
                vulkan_surface.flush();
            }
            _ => {}
        }
    });
}
