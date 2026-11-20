//! Démo et banc d'essai de la vue graphe (`lunar::graph_view`).
//!
//! ```text
//! graph_demo [--matrix <csv>] [--random N M] [--force] [--continuous]
//!            [--frames N] [--idle S] [--export-png <chemin>]
//!            [--settle ITÉRATIONS] [--size LxH] [--seed GRAINE]
//! ```
//!
//! - `--matrix <csv>` : matrice importée par le moteur (`import_matrix_file`
//!   puis `get_graph_snapshot`), labels enregistrés par `add_label` ;
//! - `--random N M` : graphe aléatoire de N nœuds et M arêtes ;
//! - `--force` : la gravité reste active ; sinon elle tourne jusqu'à
//!   stabilisation (`--settle` itérations, 600 par défaut) puis se coupe ;
//! - `--frames N` : après stabilisation, mesure N images, affiche
//!   `GRAPH_BENCH nodes= edges= fps= worst_ms=` (et `GRAPH_CPU …`) et quitte ;
//!   sans `--force`, le redessin continu est activé pour la mesure ;
//! - `--continuous` : redessine à chaque image même sans mouvement ;
//! - `--idle S` : après stabilisation (gravité coupée), compte les images et
//!   le temps processeur pendant S secondes (`GRAPH_IDLE …`) et quitte ;
//! - `--export-png <chemin>` : exporte après stabilisation et quitte.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use gpui::{
    App, Application, AsyncApp, Bounds, Context, Entity, Focusable, Render, TitlebarOptions,
    WeakEntity, Window, WindowBounds, WindowOptions, div, prelude::*, px, size,
};
use lunar::assets::Assets;
use lunar::graph_view::{GraphView, GraphViewEvent};
use lunar::platform::{force_x11_backend, install_x11_wakeup};
use lunar::theme::ActiveTheme;
use lunar::widgets::button::{Button, ButtonSize};
use lunar_engine::ipc::Channel;
use lunar_engine::{AppPaths, Lunar};
use lunar_flows_core::graph::{Edge, GraphData, Node};

/// Rendus de la fenêtre, lus par le réveil X11.
static RENDERS: AtomicU32 = AtomicU32::new(0);

/// Options de la ligne de commande.
#[derive(Clone, Debug)]
struct Args {
    /// Matrice à importer.
    matrix: Option<String>,
    /// Graphe aléatoire : nœuds, arêtes.
    random: Option<(usize, usize)>,
    /// Gravité gardée active.
    force: bool,
    /// Redessin continu.
    continuous: bool,
    /// Images à mesurer.
    frames: Option<u64>,
    /// Mesure au repos, en secondes.
    idle: Option<f64>,
    /// Export PNG.
    export_png: Option<PathBuf>,
    /// Itérations ForceAtlas2 avant stabilisation.
    settle: u64,
    /// Taille de la fenêtre.
    size: (f32, f32),
    /// Graine du graphe aléatoire.
    seed: u64,
}

/// Lit la ligne de commande.
fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        matrix: None,
        random: None,
        force: false,
        continuous: false,
        frames: None,
        idle: None,
        export_png: None,
        settle: 600,
        size: (1280.0, 800.0),
        seed: 1,
    };
    let mut it = std::env::args().skip(1);
    let value = |it: &mut std::iter::Skip<std::env::Args>, name: &str| {
        it.next().ok_or(format!("{name} : valeur attendue"))
    };
    let number = |text: String, name: &str| -> Result<u64, String> {
        text.parse().map_err(|_| format!("{name} : nombre attendu"))
    };
    while let Some(a) = it.next() {
        match a.as_str() {
            "--matrix" => args.matrix = Some(value(&mut it, "--matrix")?),
            "--random" => {
                let n = number(value(&mut it, "--random")?, "--random")? as usize;
                let m = number(value(&mut it, "--random")?, "--random")? as usize;
                args.random = Some((n, m));
            }
            "--force" => args.force = true,
            "--continuous" => args.continuous = true,
            "--frames" => args.frames = Some(number(value(&mut it, "--frames")?, "--frames")?),
            "--idle" => {
                args.idle = Some(
                    value(&mut it, "--idle")?
                        .parse()
                        .map_err(|_| "--idle : secondes attendues")?,
                );
            }
            "--export-png" => args.export_png = Some(value(&mut it, "--export-png")?.into()),
            "--settle" => args.settle = number(value(&mut it, "--settle")?, "--settle")?,
            "--seed" => args.seed = number(value(&mut it, "--seed")?, "--seed")?,
            "--size" => {
                let text = value(&mut it, "--size")?;
                let (w, h) = text.split_once('x').ok_or("--size : LxH attendu")?;
                args.size = (
                    w.parse().map_err(|_| "--size : largeur")?,
                    h.parse().map_err(|_| "--size : hauteur")?,
                );
            }
            other => return Err(format!("option inconnue : {other}")),
        }
    }
    Ok(args)
}

/// Générateur pseudo-aléatoire minimal (xorshift64*).
struct XorShift(u64);

impl XorShift {
    /// Entier suivant.
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// Entier dans `0..n`.
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Graphe aléatoire : quelques grappes (sous-réseaux), protocoles et volumes
/// variés, comme une petite capture.
fn random_graph(nodes: usize, edges: usize, seed: u64) -> GraphData {
    const COLORS: [&str; 4] = ["#8BC34A", "#2196F3", "#FFC107", "#9C27B0"];
    const PROTOCOLS: [&str; 10] = [
        "Tcp",
        "UDP",
        "DNS",
        "TLS",
        "ARP",
        "ModbusTCP",
        "S7Comm",
        "Profinet",
        "LLDP",
        "QUIQ",
    ];
    let mut rng = XorShift(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut data = GraphData::new();
    let clusters = (nodes / 40).max(1);
    for i in 0..nodes {
        let id = format!("n{i:05}");
        let cluster = i % clusters;
        let ip = format!("10.{}.{}.{}", cluster % 250, (i / 250) % 250, i % 250 + 1);
        let color = COLORS[cluster % COLORS.len()];
        let mut node = Node::new(&id, ip.clone(), String::new(), color, ip, None, None);
        node.id = id.clone();
        data.nodes.insert(id, node);
    }
    for k in 0..edges {
        let a = rng.below(nodes);
        // Arêtes surtout internes aux grappes.
        let b = if rng.below(10) < 8 {
            let base = a % clusters;
            let span = nodes / clusters;
            (base + clusters * rng.below(span.max(1))) % nodes
        } else {
            rng.below(nodes)
        };
        if a == b {
            continue;
        }
        let protocol = PROTOCOLS[rng.below(PROTOCOLS.len())];
        let bytes = 64u64 << rng.below(20);
        let edge = Edge::new(&format!("e{k}"), format!("n{a:05}"), format!("n{b:05}"))
            .with_label(protocol.to_string())
            .with_traffic(1 + rng.below(1000) as u64, bytes);
        data.edges.insert(format!("e{k:06}"), edge);
    }
    data
}

/// Matrice importée par le moteur.
fn load_matrix(path: &str) -> Result<(Arc<Lunar>, GraphData, tempfile::TempDir), String> {
    let dir = tempfile::tempdir().map_err(|e| format!("dossier temporaire : {e}"))?;
    let lunar = Lunar::new(AppPaths::under(dir.path())).map_err(|e| format!("moteur : {e}"))?;
    lunar
        .import_matrix_file(path.to_string(), Channel::new(|_| {}))
        .map_err(|e| format!("import de {path} : {e}"))?;
    let data = lunar
        .get_graph_snapshot()
        .map_err(|e| format!("snapshot : {e}"))?;
    Ok((Arc::new(lunar), data, dir))
}

/// Temps processeur du processus (utilisateur + système), en secondes.
fn process_cpu_seconds() -> Option<f64> {
    stat_cpu_seconds("/proc/self/stat")
}

/// Temps processeur du fil principal (celui de gpui), en secondes : le
/// tramage de lavapipe et ForceAtlas2 tournent sur d'autres fils.
fn ui_thread_cpu_seconds() -> Option<f64> {
    stat_cpu_seconds(&format!("/proc/self/task/{}/stat", std::process::id()))
}

/// Temps processeur de chaque fil : identifiant, nom, secondes.
fn thread_cpu_seconds() -> Vec<(String, String, f64)> {
    let Ok(dir) = std::fs::read_dir("/proc/self/task") else {
        return Vec::new();
    };
    dir.filter_map(|entry| {
        let tid = entry.ok()?.file_name().to_string_lossy().to_string();
        let name = std::fs::read_to_string(format!("/proc/self/task/{tid}/comm")).ok()?;
        let t = stat_cpu_seconds(&format!("/proc/self/task/{tid}/stat"))?;
        Some((tid, name.trim().to_string(), t))
    })
    .collect()
}

/// Temps `utime + stime` d'un fichier `stat` de proc(5), en secondes.
fn stat_cpu_seconds(path: &str) -> Option<f64> {
    let stat = std::fs::read_to_string(path).ok()?;
    let rest = stat.rsplit_once(')')?.1;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // Champs 14 et 15 de proc(5), décalés de 2 après le nom.
    let utime: f64 = fields.get(11)?.parse().ok()?;
    let stime: f64 = fields.get(12)?.parse().ok()?;
    Some((utime + stime) / 100.0)
}

/// Fenêtre de la démo : barre d'outils de l'application et vue graphe.
struct Demo {
    /// Vue graphe.
    graph: Entity<GraphView>,
    /// Moteur, avec `--matrix`.
    engine: Option<Arc<Lunar>>,
    /// Dossier du moteur, gardé jusqu'à la fermeture.
    _engine_dir: Option<tempfile::TempDir>,
    /// Abonnements : événements de la vue, changements de la fenêtre.
    _subscriptions: Vec<gpui::Subscription>,
}

/// Trace un changement d'état de la fenêtre (chacun la redessine) : sert à
/// expliquer une image inattendue pendant la mesure au repos.
fn trace_window(what: &str) {
    let since = START.get().map_or(0, |s| s.elapsed().as_millis());
    println!("GRAPH_WINDOW {what} since_start_ms={since}");
}

impl Demo {
    /// Fenêtre et graphe chargé.
    fn new(args: &Args, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let graph = cx.new(|cx| GraphView::new(window, cx));
        let mut engine = None;
        let mut engine_dir = None;
        let data = if let Some(path) = &args.matrix {
            match load_matrix(path) {
                Ok((lunar, data, dir)) => {
                    engine = Some(lunar);
                    engine_dir = Some(dir);
                    data
                }
                Err(err) => {
                    eprintln!("{err}");
                    GraphData::new()
                }
            }
        } else {
            let (n, m) = args.random.unwrap_or((200, 400));
            random_graph(n, m, args.seed)
        };
        let nodes = data.nodes.len();
        let edges = data.edges.len();
        graph.update(cx, |view, cx| view.load_snapshot(data, cx));
        println!("GRAPH_LOADED nodes={nodes} edges={edges}");

        let subscription = cx.subscribe(&graph, |demo, graph, event, cx| match event {
            GraphViewEvent::LabelCommitted {
                node_id,
                mac,
                ip,
                label,
            } => {
                println!("GRAPH_LABEL node={node_id} label={label:?}");
                let result = match &demo.engine {
                    Some(lunar) => lunar.add_label(mac.clone(), ip.clone(), label.clone()),
                    None => Ok(Vec::new()),
                };
                graph.update(cx, |view, cx| match result {
                    Ok(updates) => {
                        view.apply_updates(updates, cx);
                        view.confirm_label_edit(true, cx);
                    }
                    Err(err) => {
                        eprintln!("label refusé : {err}");
                        view.confirm_label_edit(false, cx);
                    }
                });
            }
            GraphViewEvent::ForceStateChanged(on) => {
                println!("GRAPH_FORCE on={on}");
                cx.notify();
            }
            GraphViewEvent::MatrixLabelsRequested => println!("GRAPH_MATRIX_LABELS"),
            GraphViewEvent::LabelEditRequested { node_id, current } => {
                println!("GRAPH_SELECT node={node_id} label={current:?}");
            }
        });
        Self {
            graph,
            engine,
            _engine_dir: engine_dir,
            _subscriptions: vec![
                subscription,
                cx.observe_window_activation(window, |_, window, _| {
                    trace_window(&format!("active={}", window.is_window_active()));
                }),
                cx.observe_window_appearance(window, |_, _, _| trace_window("appearance")),
                cx.observe_window_bounds(window, |_, _, _| trace_window("bounds")),
            ],
        }
    }
}

impl Render for Demo {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        RENDERS.fetch_add(1, Ordering::Relaxed);
        let theme = cx.theme();
        let on = self.graph.read(cx).is_force_running();
        let (export, toggle) = (self.graph.clone(), self.graph.clone());
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg_app)
            .text_color(theme.text)
            .font_family(theme.font_family.clone())
            .text_size(theme.text_md)
            .child(
                div()
                    .flex()
                    .gap(px(10.))
                    .p(px(8.))
                    .bg(theme.bg_bar)
                    .child(
                        Button::new("demo-export", "⬇️ Export PNG")
                            .size(ButtonSize::Small)
                            .tooltip("Exporter en PNG")
                            .on_click(move |_, _, cx| {
                                let secs = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map_or(0, |d| d.as_secs());
                                let path =
                                    PathBuf::from(format!("{secs}_network_graph_DR_Matrice.png"));
                                let task =
                                    export.update(cx, |view, cx| view.export_png(path.clone(), cx));
                                cx.spawn(async move |_| match task.await {
                                    Ok(()) => println!("GRAPH_EXPORT path={}", path.display()),
                                    Err(err) => eprintln!("export : {err}"),
                                })
                                .detach();
                            }),
                    )
                    .child(
                        Button::new(
                            "demo-force",
                            if on { "Gravité: ON" } else { "Gravité: OFF" },
                        )
                        .size(ButtonSize::Small)
                        .toggled(on)
                        .on_click(move |_, _, cx| {
                            toggle.update(cx, |view, cx| view.toggle_force(cx));
                        }),
                    ),
            )
            .child(div().flex_1().min_h_0().child(self.graph.clone()))
    }
}

/// Pilote du banc d'essai : stabilisation, puis mesure ou export.
async fn drive(args: Args, graph: WeakEntity<GraphView>, cx: &mut AsyncApp) {
    let tick = Duration::from_millis(50);
    let started = Instant::now();
    let read = |cx: &mut AsyncApp| {
        graph.read_with(cx, |view, _| {
            (
                view.stats(),
                view.model().node_count(),
                view.model().edge_count(),
            )
        })
    };

    // Stabilisation : `settle` itérations ForceAtlas2 (30 s au plus).
    loop {
        cx.background_executor().timer(tick).await;
        let Ok((stats, _, _)) = read(cx) else { return };
        if stats.fa2_iterations >= args.settle
            || started.elapsed() > Duration::from_secs(30)
            || !stats.fa2_running && stats.fa2_iterations > 0
        {
            break;
        }
    }
    let settled_after = started.elapsed();
    if !args.force {
        let _ = graph.update(cx, |view, cx| {
            if view.is_force_running() {
                view.toggle_force(cx);
            }
        });
    }
    let continuous = args.continuous || (args.frames.is_some() && !args.force);
    let _ = graph.update(cx, |view, cx| {
        view.set_continuous_redraw(continuous, cx);
        view.reset_stats();
    });
    println!(
        "GRAPH_SETTLED after_ms={} force={}",
        settled_after.as_millis(),
        args.force
    );

    if let Some(path) = args.export_png.clone() {
        // Laisse passer une image après l'arrêt de la gravité.
        cx.background_executor()
            .timer(Duration::from_millis(300))
            .await;
        let t0 = Instant::now();
        let Ok(task) = graph.update(cx, |view, cx| view.export_png(path.clone(), cx)) else {
            return;
        };
        match task.await {
            Ok(()) => println!(
                "GRAPH_EXPORT path={} ms={}",
                path.display(),
                t0.elapsed().as_millis()
            ),
            Err(err) => eprintln!("GRAPH_EXPORT_ERROR {err}"),
        }
        if args.frames.is_none() && args.idle.is_none() {
            let _ = cx.update(|cx| cx.quit());
            return;
        }
    }

    if let Some(target) = args.frames {
        let fa2_start = read(cx).map(|r| r.0).unwrap_or_default();
        let ui0 = ui_thread_cpu_seconds();
        let t0 = Instant::now();
        loop {
            cx.background_executor().timer(tick).await;
            let Ok((stats, nodes, edges)) = read(cx) else {
                return;
            };
            if stats.frames >= target || t0.elapsed() > Duration::from_secs(120) {
                let secs = t0.elapsed().as_secs_f64();
                println!(
                    "GRAPH_BENCH nodes={nodes} edges={edges} fps={:.1} worst_ms={:.1}",
                    stats.fps, stats.worst_frame_ms
                );
                println!(
                    "GRAPH_CPU frames={} cpu_ms={:.2} worst_cpu_ms={:.2} build_ms={:.2} paint_ms={:.2} edges_ms={:.2} nodes_ms={:.2} labels_ms={:.2} triangles={}",
                    stats.frames,
                    stats.cpu_ms,
                    stats.worst_cpu_ms,
                    stats.build_ms,
                    stats.paint_ms,
                    stats.edges_ms,
                    stats.nodes_ms,
                    stats.labels_ms,
                    stats.triangles
                );
                let iterations = stats.fa2_iterations - fa2_start.fa2_iterations;
                let publications = stats.fa2_publications - fa2_start.fa2_publications;
                if let (Some(a), Some(b)) = (ui0, ui_thread_cpu_seconds()) {
                    println!(
                        "GRAPH_UI_THREAD cpu_ms_per_frame={:.2} busy_pct={:.0}",
                        (b - a) * 1000.0 / stats.frames.max(1) as f64,
                        (b - a) / secs * 100.0
                    );
                }
                println!(
                    "GRAPH_FA2 running={} it_per_s={:.1} it_ms={:.2} publications_per_s={:.1}",
                    stats.fa2_running,
                    iterations as f64 / secs,
                    stats.fa2_ms,
                    publications as f64 / secs
                );
                break;
            }
        }
    }

    if let Some(seconds) = args.idle {
        // Dernière image après l'arrêt de la gravité (positions finales) :
        // la mesure commence après 1 s sans image (10 s au plus).
        let quiet_since = Instant::now();
        let mut last = (read(cx).map_or(0, |r| r.0.frames), Instant::now());
        while last.1.elapsed() < Duration::from_secs(1)
            && quiet_since.elapsed() < Duration::from_secs(10)
        {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            let frames = read(cx).map_or(0, |r| r.0.frames);
            if frames != last.0 {
                last = (frames, Instant::now());
            }
        }
        let _ = graph.update(cx, |view, _| view.reset_stats());
        let cpu0 = process_cpu_seconds();
        let threads0 = thread_cpu_seconds();
        let publications0 = read(cx).map_or(0, |r| r.0.fa2_publications);
        let t0 = Instant::now();
        let mut seen = 0;
        while t0.elapsed().as_secs_f64() < seconds {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            let frames = read(cx).map_or(0, |r| r.0.frames);
            if frames != seen {
                println!(
                    "GRAPH_IDLE_FRAME at_ms={} since_start_ms={} frames={frames}",
                    t0.elapsed().as_millis(),
                    START.get().map_or(0, |s| s.elapsed().as_millis())
                );
                seen = frames;
            }
        }
        let cpu1 = process_cpu_seconds();
        let mut busiest: Vec<(String, f64)> = thread_cpu_seconds()
            .into_iter()
            .filter_map(|(tid, name, t)| {
                let before = threads0
                    .iter()
                    .find(|(id, _, _)| *id == tid)
                    .map_or(0.0, |x| x.2);
                (t - before > 0.0).then_some((name, t - before))
            })
            .collect();
        busiest.sort_by(|a, b| b.1.total_cmp(&a.1));
        busiest.truncate(5);
        let list: Vec<String> = busiest
            .iter()
            .map(|(name, t)| format!("{name}:{:.0}%", t / seconds * 100.0))
            .collect();
        println!("GRAPH_IDLE_THREADS {}", list.join(" "));
        let Ok((stats, _, _)) = read(cx) else { return };
        let wall = t0.elapsed().as_secs_f64();
        let cpu_pct = match (cpu0, cpu1) {
            (Some(a), Some(b)) => (b - a) / wall * 100.0,
            _ => f64::NAN,
        };
        println!(
            "GRAPH_IDLE seconds={wall:.1} frames={} cpu_pct={cpu_pct:.1} fa2_running={} fa2_publications={}",
            stats.frames,
            stats.fa2_running,
            stats.fa2_publications - publications0
        );
    }

    if args.frames.is_some() || args.idle.is_some() {
        let _ = cx.update(|cx| cx.quit());
    }
}

/// Instant du lancement (horodatage des images de la mesure au repos).
static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn main() {
    START.get_or_init(Instant::now);
    env_logger::init();
    force_x11_backend();
    let args = match parse_args() {
        Ok(args) => args,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(2);
        }
    };
    Application::new()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            lunar::widgets::init(cx);
            let bounds = Bounds::centered(None, size(px(args.size.0), px(args.size.1)), cx);
            let window_args = args.clone();
            let window = match cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("LUNAR".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| Demo::new(&window_args, window, cx)),
            ) {
                Ok(window) => window,
                Err(err) => {
                    eprintln!("ouverture de la fenêtre impossible : {err:#}");
                    cx.quit();
                    return;
                }
            };
            install_x11_wakeup(cx, &RENDERS);
            let graph = window.read_with(cx, |demo, _| demo.graph.downgrade()).ok();
            let _ = window.update(cx, |demo, window, cx| {
                let handle = demo.graph.read(cx).focus_handle(cx);
                window.focus(&handle);
            });
            if let Some(graph) = graph {
                let args = args.clone();
                cx.spawn(async move |cx| drive(args, graph, cx).await)
                    .detach();
            }
            cx.activate(true);
        });
}
