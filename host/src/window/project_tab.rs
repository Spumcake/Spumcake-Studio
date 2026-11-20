//! The window's side of the conversation column's Project tab and of a
//! request's report card: the panel is read again when it is shown or the
//! window comes back into focus, a file opens as its kind asks, and a report
//! opens as the card over a faint overlay, in place of the system opener.

use super::HostWindow;
use crate::project_panel::{PanelEvent, Scripts};
use crate::report::{CardAnswer, Placed, Report};
use eframe::egui;
use std::sync::mpsc::{Receiver, channel};

/// The report card shown, with where it sits in the canon once read.
pub(crate) struct ReportCard {
    pub report: Report,
    pub placed: Placed,
    reading: Option<Receiver<Placed>>,
}

impl HostWindow {
    /// Draws the project panel where the column's Project tab leaves room,
    /// reading the project again when it is newly shown.
    pub(super) fn draw_project_tab(&mut self, ui: &mut egui::Ui, area: egui::Rect, shown_before: bool, model_name: &dyn Fn(&str) -> String) -> Vec<PanelEvent> {
        let Some(folder) = self.project.as_ref().map(|project| project.folder.clone()) else { return Vec::new() };
        if self.project_panel.stale(ui.ctx(), &folder) || !shown_before {
            let scripts = self.project_scripts().and_then(|(_, env)| Some(Scripts { scripts: env.scripts?, vars: env.vars }));
            self.project_panel.refresh(ui.ctx(), &folder, scripts);
        }
        let events = self.project_panel.show(ui, area, model_name, &self.previews.shown);
        let wanted = std::mem::take(&mut self.project_panel.wanted);
        self.ask_file_previews(&wanted);
        events
    }

    /// Does what the user asked of the panel.
    pub(super) fn project_panel_events(&mut self, events: Vec<PanelEvent>) {
        let Some(folder) = self.project.as_ref().map(|project| project.folder.clone()) else { return };
        for event in events {
            match event {
                PanelEvent::Open(path) if path.ends_with(".html") => {
                    self.ctx.open_url(egui::OpenUrl::new_tab(folder.join(&path).display().to_string()));
                }
                PanelEvent::Open(path) => {
                    if let Err(reason) = self.open_in(&path, crate::prompt_page::media_type(&path)) {
                        self.tell(reason);
                    }
                }
                // A library file's report is the report of the request that
                // filed it, found from the file's name.
                PanelEvent::OpenReport(named) if named.contains('/') => {
                    let filed = crate::project_panel::request_of(&named, |id| crate::report::page_of(&folder, id).is_ok());
                    match filed {
                        Some(request) => self.open_report(&request),
                        None => self.tell(String::from("No request filed this file, so there is no report to show.")),
                    }
                }
                PanelEvent::OpenReport(request) => self.open_report(&request),
                PanelEvent::ShowInFolder(path) => {
                    let parent = folder.join(&path).parent().map(std::path::Path::to_path_buf).unwrap_or(folder.clone());
                    self.ctx.open_url(egui::OpenUrl::new_tab(parent.display().to_string()));
                }
            }
        }
    }

    /// Shows a request's report as the card, from `requests/` or, for
    /// research, `sources/`. A report whose record cannot be read says why,
    /// and its page opens with the system's opener.
    pub(super) fn show_report(&mut self, request: &str) {
        let Some(folder) = self.project.as_ref().map(|project| project.folder.clone()) else {
            return self.tell(String::from("No project is open, so there is no report to show."));
        };
        let path = match crate::report::page_of(&folder, request) {
            Ok(path) => path,
            Err(reason) => return self.tell(reason),
        };
        let report = match Report::read(&folder, &path) {
            Ok(report) => report,
            Err(reason) => {
                self.tell(reason);
                self.ctx.open_url(egui::OpenUrl::new_tab(folder.join(&path).display().to_string()));
                return;
            }
        };
        let mut files: Vec<String> = report.outputs.clone();
        files.extend(report.references.iter().filter_map(|reference| reference.file.clone()));
        self.ask_file_previews(&files);
        let reading = self.project_scripts().and_then(|(folder, env)| {
            let scripts = env.scripts?;
            let (send, reading) = channel();
            let (report, ctx) = (report.clone(), self.ctx.clone());
            std::thread::spawn(move || {
                let _ = send.send(Placed::read(&folder, &scripts, &env.vars, &report));
                ctx.request_repaint();
            });
            Some(reading)
        });
        self.report_card = Some(ReportCard { report, placed: Placed::default(), reading });
    }

    /// Draws the report card while one is shown, and does what was asked.
    pub(super) fn draw_report_card(&mut self, ctx: &egui::Context, model_name: &dyn Fn(&str) -> String) {
        let Some(folder) = self.project.as_ref().map(|project| project.folder.clone()) else { return };
        let Some(card) = &mut self.report_card else { return };
        if let Some(placed) = card.reading.as_ref().and_then(|reading| reading.try_recv().ok()) {
            card.placed = placed;
            card.reading = None;
        }
        match crate::report::show_card(ctx, &folder, &card.report, &card.placed, model_name, &self.previews.shown) {
            Some(CardAnswer::Close) => self.report_card = None,
            Some(CardAnswer::Play(path)) => {
                self.report_card = None;
                if let Err(reason) = self.open_in(&path, crate::prompt_page::media_type(&path)) {
                    self.tell(reason);
                }
            }
            None => {}
        }
    }
}
