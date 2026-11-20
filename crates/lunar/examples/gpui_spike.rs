//! Validation de gpui 0.2.2 sur un poste (fenêtre, tracé massif, dialogue
//! GTK rfd). Rejoue les mesures de `docs/gpui-plateforme.md` :
//! `cargo run --release -p lunar --example gpui_spike -- --frames 60`.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use gpui::{
    App, Application, Bounds, Context, Hsla, PathBuilder, Pixels, Point, Render, SharedString,
    TitlebarOptions, Window, WindowBounds, WindowOptions, canvas, div, point, prelude::*, px, quad,
    rgb, size,
};
use lunar::platform::{force_x11_backend, install_x11_wakeup};

/// Nombre de rendus de la vue, lu par le réveil X11.
static RENDERS: AtomicU32 = AtomicU32::new(0);

/// Options de la ligne de commande.
struct Args {
    /// Nombre de nœuds animés.
    nodes: usize,
    /// Nombre d'arêtes tracées.
    edges: usize,
    /// Nombre d'images à mesurer avant de quitter (aucune limite sinon).
    frames: Option<u32>,
    /// Ouvre le dialogue de fichiers au démarrage.
    dialog: bool,
}

/// Lit `--nodes`, `--edges`, `--frames` et `--dialog`.
fn parse_args() -> Args {
    let mut args = Args {
        nodes: 5_000,
        edges: 10_000,
        frames: None,
        dialog: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--nodes" => args.nodes = it.next().and_then(|v| v.parse().ok()).unwrap_or(args.nodes),
            "--edges" => args.edges = it.next().and_then(|v| v.parse().ok()).unwrap_or(args.edges),
            "--frames" => args.frames = it.next().and_then(|v| v.parse().ok()),
            "--dialog" => args.dialog = true,
            _ => {}
        }
    }
    args
}

/// Générateur pseudo-aléatoire déterministe (xorshift), sans dépendance.
struct Rng(u64);
impl Rng {
    /// Tire un flottant dans [0, 1).
    fn next_f32(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// Vue du prototype : graphe animé et mesure du débit d'images.
struct Spike {
    /// Positions des nœuds.
    pos: Vec<(f32, f32)>,
    /// Vitesses des nœuds.
    vel: Vec<(f32, f32)>,
    /// Arêtes (indices de nœuds).
    edges: Vec<(usize, usize)>,
    /// Palette des nœuds et des arêtes.
    colors: Vec<Hsla>,
    /// Images rendues.
    frames: u32,
    /// Images à rendre avant de quitter.
    target_frames: Option<u32>,
    /// Instant du premier rendu.
    started: Option<Instant>,
    /// Pire intervalle entre deux rendus.
    worst: Duration,
    /// Instant du rendu précédent.
    last: Option<Instant>,
    /// Résultat du dialogue de fichiers.
    dialog_result: SharedString,
}

impl Spike {
    /// Construit un graphe pseudo-aléatoire déterministe.
    fn new(args: &Args) -> Self {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let pos = (0..args.nodes)
            .map(|_| (rng.next_f32() * 1800.0, rng.next_f32() * 1000.0))
            .collect();
        let vel = (0..args.nodes)
            .map(|_| (rng.next_f32() - 0.5, rng.next_f32() - 0.5))
            .collect();
        let n = args.nodes.max(1) as f32;
        let edges = (0..args.edges)
            .map(|_| {
                (
                    (rng.next_f32() * n) as usize % args.nodes.max(1),
                    (rng.next_f32() * n) as usize % args.nodes.max(1),
                )
            })
            .collect();
        let colors = [0x8BC34A, 0x2196F3, 0xFFC107, 0xE53935, 0xFF9800, 0x00BCD4]
            .iter()
            .map(|c| rgb(*c).into())
            .collect();
        Self {
            pos,
            vel,
            edges,
            colors,
            frames: 0,
            target_frames: args.frames,
            started: None,
            worst: Duration::ZERO,
            last: None,
            dialog_result: "".into(),
        }
    }

    /// Avance l'animation d'un pas.
    fn step(&mut self) {
        for (p, v) in self.pos.iter_mut().zip(&self.vel) {
            p.0 += v.0;
            p.1 += v.1;
        }
    }

    /// Ouvre le dialogue de fichiers GTK (rfd) sans bloquer la boucle.
    fn open_dialog(&mut self, cx: &mut Context<Self>) {
        let task = rfd::AsyncFileDialog::new()
            .set_title("Choisir un PCAP")
            .add_filter("PCAP", &["pcap", "pcapng", "cap"])
            .pick_file();
        cx.spawn(async move |this, cx| {
            let picked = task.await;
            let text = picked
                .map(|f| f.path().display().to_string())
                .unwrap_or_else(|| "annulé".to_string());
            println!("SPIKE_DIALOG={text}");
            let _ = this.update(cx, |this, cx| {
                this.dialog_result = text.into();
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for Spike {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        if let Some(last) = self.last {
            self.worst = self.worst.max(now - last);
        }
        self.last = Some(now);
        let started = *self.started.get_or_insert(now);
        self.frames += 1;
        RENDERS.fetch_add(1, Ordering::Relaxed);
        if self.frames == 1 {
            println!("SPIKE_FIRST_FRAME");
        }
        if let Some(target) = self.target_frames
            && self.frames >= target
        {
            let secs = (now - started).as_secs_f64();
            println!(
                "SPIKE_BENCH nodes={} edges={} frames={} fps={:.1} pire_frame_ms={:.1}",
                self.pos.len(),
                self.edges.len(),
                self.frames,
                f64::from(self.frames - 1) / secs.max(1e-9),
                self.worst.as_secs_f64() * 1000.0
            );
            cx.quit();
        }
        self.step();
        window.request_animation_frame();

        let pos = self.pos.clone();
        let edges = self.edges.clone();
        let colors = self.colors.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x111111))
            .text_color(rgb(0xffffff))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .p_2()
                    .bg(rgb(0x070416))
                    .child(
                        div()
                            .id("dialog")
                            .px_3()
                            .py_1()
                            .bg(rgb(0x116466))
                            .child("Ouvrir un PCAP…")
                            .on_click(cx.listener(|this, _, _, cx| this.open_dialog(cx))),
                    )
                    .child(format!(
                        "{} nœuds · {} arêtes · frame {} · {}",
                        self.pos.len(),
                        self.edges.len(),
                        self.frames,
                        self.dialog_result
                    )),
            )
            .child(
                div().flex_1().child(
                    canvas(
                        |_, _, _| {},
                        move |bounds: Bounds<Pixels>, _, window, _| {
                            let o = bounds.origin;
                            let at = |p: (f32, f32)| -> Point<Pixels> {
                                point(o.x + px(p.0 % 1800.0), o.y + px(p.1.rem_euclid(1000.0)))
                            };
                            // Arêtes regroupées par couleur : un chemin par couleur.
                            for (ci, color) in colors.iter().enumerate() {
                                let mut b = PathBuilder::stroke(px(1.));
                                let mut any = false;
                                for (i, (s, t)) in edges.iter().enumerate() {
                                    if i % colors.len() != ci {
                                        continue;
                                    }
                                    b.move_to(at(pos[*s]));
                                    b.line_to(at(pos[*t]));
                                    any = true;
                                }
                                if any && let Ok(path) = b.build() {
                                    let mut c = *color;
                                    c.a = 0.35;
                                    window.paint_path(path, c);
                                }
                            }
                            for (i, p) in pos.iter().enumerate() {
                                let c = at(*p);
                                let r = px(4.);
                                window.paint_quad(quad(
                                    Bounds::new(point(c.x - r, c.y - r), size(r * 2., r * 2.)),
                                    r,
                                    colors[i % colors.len()],
                                    px(1.),
                                    gpui::black(),
                                    Default::default(),
                                ));
                            }
                        },
                    )
                    .size_full(),
                ),
            )
    }
}

fn main() {
    env_logger::init();
    force_x11_backend();
    let args = parse_args();
    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
        let dialog = args.dialog;
        let window = match cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("LUNAR".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_, cx| cx.new(|_| Spike::new(&args)),
        ) {
            Ok(window) => window,
            Err(err) => {
                eprintln!("ouverture de la fenêtre impossible : {err:#}");
                cx.quit();
                return;
            }
        };
        // Contournement gpui 0.2.2 / X11 : voir `platform::x11_wakeup`.
        install_x11_wakeup(cx, &RENDERS);
        if dialog {
            let _ = window.update(cx, |spike, _, cx| spike.open_dialog(cx));
        }
        cx.activate(true);
    });
}
