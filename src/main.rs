//! GUI binary: the rendered, interactive app. The simulation core lives in the
//! `econ_sim` library; this file is the rendering/UI/input layer (gated by `gui`).

use bevy::ecs::schedule::ScheduleLabel;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::input::touch::Touch;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use econ_sim::economy::{self, EconStats, HungerControl};
use econ_sim::goods::{self, GoodForm};
use econ_sim::movement::{self, tile_to_pixel};
use econ_sim::noot::{
    Action, Claim, Hunger, Inventory, Noot, NootMeta, NootName, TilePos, Trader, Wallet,
    EXPLORE_MAX, EXPLORE_MIN, STARTING_BUCKS,
};
use econ_sim::policy::{ActorCritic, PolicyMemory, Trainer};
use econ_sim::world::{generate, World};
use econ_sim::history::RollupHistory;
use econ_sim::{elements, graph, hex, icon, rng::Rng, save, MapView, Sim, SimRng};

// --- World generation knobs -------------------------------------------------
const COLS: i32 = 30;
const ROWS: i32 = 22;
const HEX_SIZE: f32 = 26.0;

// --- Population -------------------------------------------------------------
/// Total noots at a fresh start. All spawn claimless and free-roaming — anyone can
/// become a miner by claiming a deposit it wanders onto (no pre-seeded miners).
const N_NOOTS: usize = 56;

// --- Camera limits ----------------------------------------------------------
const MIN_ZOOM: f32 = 0.3;
const MAX_ZOOM: f32 = 8.0;

// --- Selection / follow -----------------------------------------------------
/// Max screen-pixels a touch may move and still count as a tap (not a pan).
const TAP_SLOP: f32 = 12.0;
/// Min single-finger pan delta (screen px) that releases the follow lock.
const DESELECT_PAN_SLOP: f32 = 1.5;

// --- Top-right button column layout (shared by spawn_ui and the pick guard) -
// A horizontal transport bar — [<<] [Play/Pause] [>>] and a ticks/s readout — sits at
// the top; the overlay toggles stack in a narrower column below it.
const PAUSE_BTN_W: f32 = 120.0;
const PAUSE_BTN_H: f32 = 44.0;
const PAUSE_BTN_MARGIN: f32 = 10.0;
const BTN_GAP: f32 = 8.0;

// Transport bar: two side buttons (slower / faster), a wider middle play/pause button,
// then the ticks/s readout, all in one right-anchored row of height `PAUSE_BTN_H`.
const XPORT_SIDE_W: f32 = 46.0;
const XPORT_MID_W: f32 = 60.0;
const XPORT_READOUT_W: f32 = 104.0;
const XPORT_GAP: f32 = 6.0;
const XPORT_BAR_W: f32 = XPORT_SIDE_W * 2.0 + XPORT_MID_W + XPORT_READOUT_W + XPORT_GAP * 3.0;

// The toggle column begins one transport-bar height below the top margin, so its first
// button lines up just under the transport bar.
const OVERLAY_BTN_TOP: f32 = PAUSE_BTN_MARGIN + PAUSE_BTN_H + BTN_GAP;
const NOOT_BTN_TOP: f32 = OVERLAY_BTN_TOP + PAUSE_BTN_H + BTN_GAP;
const SAVE_BTN_TOP: f32 = NOOT_BTN_TOP + PAUSE_BTN_H + BTN_GAP;
const NEW_BTN_TOP: f32 = SAVE_BTN_TOP + PAUSE_BTN_H + BTN_GAP;
const GRAPHS_BTN_TOP: f32 = NEW_BTN_TOP + PAUSE_BTN_H + BTN_GAP;
const PRICES_BTN_TOP: f32 = GRAPHS_BTN_TOP + PAUSE_BTN_H + BTN_GAP;
const WEALTH_BTN_TOP: f32 = PRICES_BTN_TOP + PAUSE_BTN_H + BTN_GAP;
/// Bottom edge of the toggle column (taps above this in the column are UI, not map).
const BTN_COLUMN_BOTTOM: f32 = WEALTH_BTN_TOP + PAUSE_BTN_H;

// --- Graphs overlay ---------------------------------------------------------
/// Size of the big correlation (overlay) chart texture, and of each per-stat sparkline.
const OVERLAY_W: u32 = 380;
const OVERLAY_H: u32 = 170;
const SPARK_W: u32 = 104;
const SPARK_H: u32 = 34;
/// How often (real seconds) a graph sample is taken. Retention is whole-run (rolled up
/// to a bounded size; see `econ_sim::history`).
const GRAPH_SAMPLE_SECS: f32 = 0.25;
/// Cell background in the graphs grid: dim when a stat is off the overlay, bright when
/// it's been tapped into the correlation chart.
const GRAPH_CELL_OFF: Color = Color::srgba(0.10, 0.10, 0.13, 0.9);
const GRAPH_CELL_ON: Color = Color::srgba(0.20, 0.28, 0.40, 0.95);

const BTN_OFF: Color = Color::srgba(0.12, 0.12, 0.12, 0.85);
const VALUE_BTN_ON: Color = Color::srgba(0.62, 0.22, 0.16, 0.9);
const TERRAIN_BTN_ON: Color = Color::srgba(0.20, 0.45, 0.30, 0.9);
const TRADES_BTN_ON: Color = Color::srgba(0.70, 0.55, 0.12, 0.9);
const ROUTES_BTN_ON: Color = Color::srgba(0.18, 0.42, 0.62, 0.9);
const ROADS_BTN_ON: Color = Color::srgba(0.55, 0.40, 0.20, 0.9);
/// Confirmation tint + how long (real seconds) the Save button flashes after a save.
const SAVE_FLASH_COLOR: Color = Color::srgba(0.20, 0.55, 0.30, 0.95);
const SAVE_FLASH_SECS: f32 = 1.0;

/// How long after launch the camera keeps re-fitting the map, so the wasm canvas
/// (which resizes to its parent post-launch) settles before zoom is left to the user.
const FIT_SETTLE_SECS: f32 = 0.3;

/// The noot currently selected and followed, if any.
#[derive(Resource, Default)]
struct Selection(Option<Entity>);

/// The map tile the player tapped to inspect (terrain / deposit / structure), if any.
/// Mutually exclusive with `Selection`: a tap picks either a noot to follow or a hex.
#[derive(Resource, Default)]
struct SelectedHex(Option<usize>);

/// When true the simulation systems are frozen; input/camera/HUD keep running.
#[derive(Resource, Default)]
struct Paused(bool);

/// The fixed-tick simulation schedule. Its systems advance the world by exactly one
/// `economy::TICK_DT` per run; `run_sim_ticks` runs it many times per rendered frame
/// (per `SimSpeed`), decoupling sim rate from render rate.
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
struct SimSchedule;

/// System set holding the sim driver, so the per-frame visual/HUD systems can order
/// themselves after it (they read the freshest sim state each frame).
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
struct SimDriver;

/// Selectable game speeds in **ticks per second**, stepped by the transport bar's
/// `<<` / `>>` buttons (default 60 t/s ≈ real-time). Higher values run the sim faster
/// than the wall clock while the render loop stays at the browser's frame rate.
const SPEED_STEPS: [f32; 6] = [30.0, 60.0, 120.0, 240.0, 480.0, 960.0];
/// Index into `SPEED_STEPS` the sim starts at (60 t/s).
const SPEED_DEFAULT_IDX: usize = 1;

/// Hard cap on sim ticks executed in a single rendered frame, so a slow frame (or a
/// background tab catching up) can't spiral into an unbounded tick burst.
const MAX_TICKS_PER_FRAME: u32 = 1500;

/// How fast the sim advances, decoupled from render. `accumulator` carries the
/// fractional tick remainder between frames so the average rate matches exactly.
#[derive(Resource)]
struct SimSpeed {
    ticks_per_second: f32,
    accumulator: f32,
}

impl Default for SimSpeed {
    fn default() -> Self {
        Self {
            ticks_per_second: SPEED_STEPS[SPEED_DEFAULT_IDX],
            accumulator: 0.0,
        }
    }
}

/// Drive the fixed-tick `SimSchedule` from real time: bank `ticks_per_second · dt`
/// ticks worth of work and run that many (capped). Frozen while paused. Exclusive so
/// it can run a whole schedule per tick. (`World` here is Bevy's ECS world — the game
/// map type `econ_sim::world::World` shadows the bare name in this module.)
fn run_sim_ticks(world: &mut bevy::prelude::World) {
    if world.resource::<Paused>().0 {
        return;
    }
    let dt = world.resource::<Time>().delta_secs();
    let n = {
        let mut speed = world.resource_mut::<SimSpeed>();
        speed.accumulator += speed.ticks_per_second * dt;
        let whole = speed.accumulator.floor();
        speed.accumulator -= whole; // keep only the sub-tick remainder (no backlog)
        (whole as u32).min(MAX_TICKS_PER_FRAME)
    };
    for _ in 0..n {
        world.run_schedule(SimSchedule);
    }
}

/// The on-screen pause toggle and its caption.
#[derive(Component)]
struct PauseButton;
#[derive(Component)]
struct PauseLabel;

/// Cycles the map overlay (none → terrain → trades), touch equivalent of the V key.
#[derive(Component)]
struct MapOverlayButton;
/// Caption on the map-overlay button, kept in sync with the active mode.
#[derive(Component)]
struct MapOverlayLabel;
/// Cycles the noot-colouring mode (touch equivalent of the N key).
#[derive(Component)]
struct NootColorButton;
/// Clears the save and rerolls a fresh world (touch equivalent of the G key).
#[derive(Component)]
struct NewWorldButton;
/// Snapshots the full game state to localStorage (touch equivalent of the S key).
#[derive(Component)]
struct SaveButton;
/// Caption on the Save button; briefly flips to "Saved!" after a save fires.
#[derive(Component)]
struct SaveLabel;
/// Transport buttons: step the simulation speed down / up through `SPEED_STEPS`.
#[derive(Component)]
struct SpeedDownButton;
#[derive(Component)]
struct SpeedUpButton;
/// The `x.x ticks/s` readout in the transport bar, kept in sync with the active speed.
#[derive(Component)]
struct SpeedLabel;
/// Caption on the noot-colouring button, kept in sync with the active mode.
#[derive(Component)]
struct NootColorLabel;

/// How noots are tinted on the map. Ownership is the default; the rest rank the
/// population on a property and scale white (low) → blue (high).
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum NootColorMode {
    #[default]
    Ownership,
    Age,
    Bucks,
    Positional,
    Transactions,
}

impl NootColorMode {
    fn next(self) -> Self {
        match self {
            Self::Ownership => Self::Age,
            Self::Age => Self::Bucks,
            Self::Bucks => Self::Positional,
            Self::Positional => Self::Transactions,
            Self::Transactions => Self::Ownership,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Ownership => "Noots: owner",
            Self::Age => "Noots: age",
            Self::Bucks => "Noots: bucks",
            Self::Positional => "Noots: posit",
            Self::Transactions => "Noots: trades",
        }
    }
}

#[derive(Resource, Default)]
struct NootColoring(NootColorMode);

/// The single map heatmap shown, cycled by the Overlay button: none, terrain
/// difficulty, trade density, or movement (route) density. (The old crowd-density
/// overlay was dropped.)
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum MapOverlayMode {
    #[default]
    None,
    Terrain,
    Trades,
    Routes,
    Roads,
}

impl MapOverlayMode {
    fn next(self) -> Self {
        match self {
            Self::None => Self::Terrain,
            Self::Terrain => Self::Trades,
            Self::Trades => Self::Routes,
            Self::Routes => Self::Roads,
            Self::Roads => Self::None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::None => "Overlay: off",
            Self::Terrain => "Overlay: terrain",
            Self::Trades => "Overlay: trades",
            Self::Routes => "Overlay: routes",
            Self::Roads => "Overlay: roads",
        }
    }
}

/// Text of the bottom panel describing the selected noot.
#[derive(Component)]
struct SelectionText;

/// The highlight ring drawn around the selected noot.
#[derive(Component)]
struct SelectionRing;

/// The highlight ring drawn around the tapped (inspected) hex.
#[derive(Component)]
struct HexHighlight;

/// Which inspection overlays are currently shown. `map` is the cycled map heatmap
/// (none / terrain / trades / routes); `strip` is the always-on top sparkline strip (shown
/// by default, collapsible); `graphs`/`prices` are the on-demand chart panels.
#[derive(Resource)]
struct Overlays {
    /// The map heatmap currently shown (cycled by the Overlay button).
    map: MapOverlayMode,
    strip: bool,
    graphs: bool,
    prices: bool,
    wealth: bool,
}

impl Default for Overlays {
    fn default() -> Self {
        Self {
            map: MapOverlayMode::None,
            strip: true,
            graphs: false,
            prices: false,
            wealth: false,
        }
    }
}

/// Whole-run history of every graphed stat (rolled up: recent full-res, older
/// downsampled), oldest → newest. Persisted across save/reload.
#[derive(Resource, Default)]
struct StatHistory(RollupHistory<{ graph::N_SERIES }>);

/// Whole-run history of each resource's last sale price (held through no-trade spells),
/// rolled up like `StatHistory` — feeds the per-resource price graphs.
#[derive(Resource, Default)]
struct PriceHistory(RollupHistory<{ goods::N_ITEMS }>);

/// Which stats are currently drawn together on the correlation (overlay) chart.
#[derive(Resource)]
struct GraphSelection([bool; graph::N_SERIES]);

impl Default for GraphSelection {
    fn default() -> Self {
        // Start by overlaying production and consumption — the canonical paired series.
        let mut sel = [false; graph::N_SERIES];
        sel[0] = true;
        sel[1] = true;
        Self(sel)
    }
}

/// Handles to the chart textures: one big overlay chart plus one sparkline per stat.
#[derive(Resource, Clone)]
struct GraphAssets {
    overlay: Handle<Image>,
    sparks: Vec<Handle<Image>>,
    /// One price sparkline texture per resource item (the Prices panel).
    price_sparks: Vec<Handle<Image>>,
    /// The sorted money-per-noot distribution chart (the Wealth panel).
    wealth: Handle<Image>,
}

/// Root of the correlation-chart panel (shown/hidden by the Graphs toggle).
#[derive(Component)]
struct GraphsPanel;
/// The on-screen Graphs toggle button (opens the correlation chart).
#[derive(Component)]
struct GraphsButton;
/// Root of the per-resource price-chart panel (shown/hidden by the Prices toggle).
#[derive(Component)]
struct PricesPanel;
/// The on-screen Prices toggle button (opens the per-resource price graphs).
#[derive(Component)]
struct PricesButton;
/// The caption above a resource's price sparkline (`"<name> ₦<price>"`).
#[derive(Component)]
struct PriceLabel {
    item: usize,
}
/// Root of the wealth-distribution panel (shown/hidden by the Wealth toggle).
#[derive(Component)]
struct WealthPanel;
/// The on-screen Wealth toggle button (opens the sorted money-per-noot chart).
#[derive(Component)]
struct WealthButton;
/// Caption above the wealth chart, showing the current Gini coefficient.
#[derive(Component)]
struct WealthLabel;
/// The collapsible body of the top sparkline strip (the row of stat cells).
#[derive(Component)]
struct StatStripBody;
/// The strip's collapse/expand toggle button, and its caption.
#[derive(Component)]
struct StripToggle;
#[derive(Component)]
struct StripToggleLabel;
/// A tappable per-stat cell; tapping toggles the stat onto the correlation chart.
#[derive(Component)]
struct GraphCell {
    series: usize,
}
/// The value caption inside a stat cell (`"<label>: <latest>"`).
#[derive(Component)]
struct GraphLabel {
    series: usize,
}

/// Per-hex tint cell for the terrain-difficulty overlay (static colour).
#[derive(Component)]
struct TerrainOverlay;

/// Per-hex heat cell for the trade-density overlay (`tile` is `row * cols + col`);
/// recoloured from the cumulative `EconStats::trade_hexes` heatmap.
#[derive(Component)]
struct TradeOverlay {
    tile: usize,
}

/// Per-hex heat cell for the movement (route) overlay (`tile` is `row * cols + col`);
/// recoloured from the cumulative `EconStats::traffic_hexes` heatmap.
#[derive(Component)]
struct RouteOverlay {
    tile: usize,
}

/// Per-hex heat cell for the live **road** overlay (`tile` is `row * cols + col`);
/// recoloured from the decaying `World::road` field — the current desire-path network,
/// as distinct from `RouteOverlay`'s all-time cumulative traffic.
#[derive(Component)]
struct RoadOverlay {
    tile: usize,
}

/// Ring drawn around a deposit while it is claimed.
#[derive(Component)]
struct DepositOutline {
    deposit: usize,
}

/// The kind-coloured **body** hex of a noot-built structure (cyan = shop, orange =
/// refinery), carrying its `World::structures` index so `sync_structure_markers` can
/// recolour it if a build-over changes the kind. Each structure also gets a dark frame hex
/// (untagged) and a white [`StructureEmblem`] on top. Structures are created during play,
/// so markers spawn incrementally rather than all at setup.
#[derive(Component)]
struct StructureMarker {
    structure: usize,
}

/// The white emblem on top of a structure that signals its kind by *shape* — an upward
/// triangle (shop) or a diamond (refinery) — so the two read apart at a glance, not by
/// colour alone. `sync_structure_markers` swaps the mesh if a build-over flips the kind.
#[derive(Component)]
struct StructureEmblem {
    structure: usize,
}

/// A world-space label (one of a reused pool) showing how many noots are stacked on a
/// hex, repositioned each refresh by `update_stack_labels`. Noots can share a tile and
/// their sprites overlap, so a count makes a crowded hex legible.
#[derive(Component)]
struct StackLabel;

/// Noot body colours: green while unclaimed, amber once it owns a deposit.
const NOOT_UNCLAIMED: Color = Color::srgb(0.40, 0.85, 0.45);
const NOOT_OWNER: Color = Color::srgb(0.95, 0.78, 0.25);

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "econ-sim".into(),
                canvas: Some("#bevy".into()),
                fit_canvas_to_parent: true,
                // Mobile-first: let the canvas own touch gestures instead of the
                // browser (page scroll / pinch-zoom).
                prevent_default_event_handling: true,
                ..default()
            }),
            ..default()
        }))
        .init_resource::<EconStats>()
        .init_resource::<Selection>()
        .init_resource::<SelectedHex>()
        .init_resource::<Paused>()
        .init_resource::<SimSpeed>()
        .init_resource::<Overlays>()
        .init_resource::<NootColoring>()
        .init_resource::<StatHistory>()
        .init_resource::<PriceHistory>()
        .init_resource::<GraphSelection>()
        .init_resource::<economy::IncomeControl>()
        .init_resource::<economy::PriceField>()
        .init_resource::<Trainer>()
        .add_systems(Startup, setup)
        // The fixed-tick simulation pipeline (each system advances the world by one
        // `TICK_DT`), driven many times per frame by `run_sim_ticks`. The system list
        // lives in `economy::add_sim_systems`, shared with the headless harness.
        .add_schedule({
            let mut sched = Schedule::new(SimSchedule);
            economy::add_sim_systems(&mut sched);
            sched
        })
        // Per-frame (real-time) systems: drive the sim ticks, then render/input/HUD.
        // `run_sim_ticks` is exclusive and runs first so the visuals reflect the
        // freshest sim state; `movement::movement` glides sprites toward their tiles.
        .add_systems(Update, run_sim_ticks.in_set(SimDriver))
        .add_systems(
            Update,
            (
                (
                    movement::movement,
                    pick_selection,
                    touch_camera,
                    keyboard_mouse_camera,
                    follow_selected,
                    pause_controls,
                    overlay_controls,
                    save_game,
                    new_world_controls,
                    speed_controls,
                    graphs_controls,
                    fit_camera_to_screen,
                    prices_controls,
                    wealth_controls,
                ),
                (
                    update_selection_ring,
                    update_hex_highlight,
                    update_selection_panel,
                    update_noot_color,
                    update_trade_overlay,
                    update_route_overlay,
                    update_road_overlay,
                    update_deposit_outlines,
                    sync_structure_markers,
                    update_stack_labels,
                    sample_stats,
                    render_graphs,
                    render_prices,
                    render_wealth,
                    graph_select,
                    strip_controls,
                    hide_loading_screen,
                ),
            )
                .after(SimDriver),
        )
        .run();
}

/// Draw a fresh world seed each load so every visit differs. The run stays
/// reproducible from this value (the HUD prints it).
fn random_seed() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        let a = js_sys::Date::now().to_bits();
        let b = (js_sys::Math::random() * u64::MAX as f64) as u64;
        a ^ b.rotate_left(32) ^ 0x9E37_79B9_7F4A_7C15
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15)
    }
}

/// Once Bevy has produced its first frame, fade out the HTML loading overlay by
/// tagging it `ready`. Runs every frame but acts only once; a no-op off the web.
fn hide_loading_screen(mut done: Local<bool>) {
    if *done {
        return;
    }
    *done = true;
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(element) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("loading"))
        {
            let _ = element.class_list().add_1("ready");
        }
    }
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut fonts: ResMut<Assets<Font>>,
) {
    // Resume a saved run (full state) if one exists; otherwise roll a fresh world.
    // The saved controllers/stats override the defaults here; on a fresh start the
    // hunger PID gets a population-based target (and Income/EconStats keep defaults).
    let (world, restore_noots, restore_policy) = match save::load() {
        Some(snap) => {
            commands.insert_resource(snap.hunger);
            commands.insert_resource(snap.income);
            commands.insert_resource(snap.stats);
            // Restore the whole-run graph history so the charts come back populated.
            commands.insert_resource(StatHistory(snap.stat_history));
            commands.insert_resource(PriceHistory(snap.price_history));
            (snap.world, Some(snap.noots), Some(snap.policy))
        }
        None => {
            let world = generate(random_seed(), COLS, ROWS, HEX_SIZE);
            commands.insert_resource(HungerControl::new(
                economy::TARGET_DEATH_FRAC_PER_TICK * N_NOOTS as f32,
            ));
            (world, None, None)
        }
    };
    let hex_size = world.hex_size;
    let n_tiles = (world.cols * world.rows) as usize;

    // Embed a full-coverage monospace font so Unicode glyphs (₦, →, ·, —) render —
    // Bevy's built-in default font is a tiny ASCII subset that shows them as tofu.
    let ui_font = fonts.add(
        Font::try_from_bytes(include_bytes!("../assets/fonts/DejaVuSansMono.ttf").to_vec())
            .expect("embedded UI font should parse"),
    );

    // One thematic icon texture per chosen element (used on the map and in the HUD).
    let icons: [Handle<Image>; 4] =
        std::array::from_fn(|slot| images.add(icon::render_icon(world.chosen[slot].id.0)));

    // Blank chart textures for the graphs overlay (re-rasterized live by `render_graphs`).
    let graph_assets = GraphAssets {
        overlay: images.add(graph::blank_image(OVERLAY_W, OVERLAY_H)),
        sparks: (0..graph::N_SERIES)
            .map(|_| images.add(graph::blank_image(SPARK_W, SPARK_H)))
            .collect(),
        price_sparks: (0..goods::N_ITEMS)
            .map(|_| images.add(graph::blank_image(SPARK_W, SPARK_H)))
            .collect(),
        wealth: images.add(graph::blank_image(OVERLAY_W, OVERLAY_H)),
    };
    commands.insert_resource(graph_assets.clone());

    // Centre the map on the origin. `fit_camera_to_screen` does the real framing
    // once the window size is known; this is just a sane portrait fallback for the
    // first frames (fill the tighter axis — the looser one overflows and can be panned).
    let mut min = Vec2::splat(f32::MAX);
    let mut max = Vec2::splat(f32::MIN);
    for tile in &world.tiles {
        let (x, y) = hex::hex_center(tile.col, tile.row, hex_size);
        min = min.min(Vec2::new(x, y));
        max = max.max(Vec2::new(x, y));
    }
    let offset = -(min + max) * 0.5;
    let map_w = (max.x - min.x) + hex_size * 2.0;
    let map_h = (max.y - min.y) + hex_size * 2.0;
    let init_zoom = (map_w / 400.0).min(map_h / 800.0).clamp(MIN_ZOOM, MAX_ZOOM);

    commands.spawn((Camera2d, Transform::from_scale(Vec3::splat(init_zoom))));

    // Tiles share one neutral material — difficulty is shown *only* via the toggleable
    // terrain overlay. Each tile also gets three hidden heat cells stacked above it
    // (z 0.4 terrain, z 1.5 routes, z 1.6 trades) — only one shows at a time, and they sit
    // just under the noot layer (z 2.0).
    let hex_mesh = meshes.add(RegularPolygon::new(hex_size * 0.96, 6));
    let tile_mat = materials.add(Color::srgb(0.18, 0.20, 0.22));
    for tile in &world.tiles {
        let (x, y) = hex::hex_center(tile.col, tile.row, hex_size);
        let (px, py) = (x + offset.x, y + offset.y);
        commands.spawn((
            Mesh2d(hex_mesh.clone()),
            MeshMaterial2d(tile_mat.clone()),
            Transform::from_xyz(px, py, 0.0),
        ));

        // Terrain-difficulty overlay: green (easy) → red (hard), by the tile's
        // continuous difficulty. A sub-1.0 alpha makes `ColorMaterial` blend.
        let d = tile.difficulty.clamp(0.0, 1.0);
        let terr_color = Color::srgba((0.2 + 1.6 * d).min(1.0), (0.7 - 1.2 * d).max(0.0), 0.1, 0.5);
        commands.spawn((
            Mesh2d(hex_mesh.clone()),
            MeshMaterial2d(materials.add(terr_color)),
            Transform::from_xyz(px, py, 0.4),
            Visibility::Hidden,
            TerrainOverlay,
        ));

        // Trade-density heat overlay (gold), recoloured by `update_trade_overlay` from
        // the cumulative trade heatmap. Born translucent (alpha < 1) so the material is
        // created in blend mode; sits just under the noot layer (z 2.0).
        let idx = (tile.row * world.cols + tile.col) as usize;
        commands.spawn((
            Mesh2d(hex_mesh.clone()),
            MeshMaterial2d(materials.add(Color::srgba(0.98, 0.80, 0.15, 0.0))),
            Transform::from_xyz(px, py, 1.6),
            Visibility::Hidden,
            TradeOverlay { tile: idx },
        ));

        // Movement (route) heat overlay (cyan), recoloured by `update_route_overlay` from
        // the cumulative traffic heatmap. Same translucent-birth trick as the trade cell.
        commands.spawn((
            Mesh2d(hex_mesh.clone()),
            MeshMaterial2d(materials.add(Color::srgba(0.25, 0.70, 0.95, 0.0))),
            Transform::from_xyz(px, py, 1.5),
            Visibility::Hidden,
            RouteOverlay { tile: idx },
        ));

        // Live road overlay (tan), recoloured by `update_road_overlay` from the decaying
        // `World::road` field — the current desire-path network.
        commands.spawn((
            Mesh2d(hex_mesh.clone()),
            MeshMaterial2d(materials.add(Color::srgba(0.80, 0.62, 0.32, 0.0))),
            Transform::from_xyz(px, py, 1.45),
            Visibility::Hidden,
            RoadOverlay { tile: idx },
        ));
    }

    // Deposit markers: a dark backing disc carrying the element's thematic icon,
    // with a hidden claim outline ringing it.
    let disc_mesh = meshes.add(Circle::new(hex_size * 0.5));
    let outline_mesh = meshes.add(Annulus::new(hex_size * 0.54, hex_size * 0.62));
    for (di, deposit) in world.deposits.iter().enumerate() {
        let tile = &world.tiles[deposit.tile];
        let (x, y) = hex::hex_center(tile.col, tile.row, hex_size);
        let (px, py) = (x + offset.x, y + offset.y);
        commands.spawn((
            Mesh2d(disc_mesh.clone()),
            MeshMaterial2d(materials.add(Color::srgba(0.08, 0.08, 0.10, 0.88))),
            Transform::from_xyz(px, py, 1.0),
        ));
        commands.spawn((
            Sprite {
                image: icons[deposit.element_slot].clone(),
                custom_size: Some(Vec2::splat(hex_size * 0.85)),
                ..default()
            },
            Transform::from_xyz(px, py, 1.05),
        ));
        commands.spawn((
            Mesh2d(outline_mesh.clone()),
            MeshMaterial2d(materials.add(Color::srgb(0.97, 0.97, 0.92))),
            Transform::from_xyz(px, py, 1.1),
            Visibility::Hidden,
            DepositOutline { deposit: di },
        ));
    }

    // Pool of world-space stack-count labels (one per possible 2-noot pairing), parked
    // off-screen and hidden; `update_stack_labels` positions/fills the active ones.
    for _ in 0..N_NOOTS {
        commands.spawn((
            Text2d::new(""),
            TextFont {
                font: ui_font.clone(),
                font_size: 18.0,
                ..default()
            },
            TextColor(Color::srgb(1.0, 0.95, 0.4)),
            Transform::from_xyz(0.0, 0.0, 3.0),
            Visibility::Hidden,
            StackLabel,
        ));
    }

    commands.insert_resource(MapView {
        offset,
        hex_size,
        map_w,
        map_h,
    });

    // Highlight ring for the selected noot (hidden until something is picked).
    let ring_mesh = meshes.add(Annulus::new(hex_size * 0.34, hex_size * 0.46));
    commands.spawn((
        Mesh2d(ring_mesh),
        MeshMaterial2d(materials.add(Color::srgb(1.0, 0.95, 0.3))),
        Transform::from_xyz(0.0, 0.0, 2.5),
        Visibility::Hidden,
        SelectionRing,
    ));

    // Wider white ring marking the tapped (inspected) hex; sits just above the terrain.
    let hex_ring = meshes.add(Annulus::new(hex_size * 0.7, hex_size * 0.82));
    commands.spawn((
        Mesh2d(hex_ring),
        MeshMaterial2d(materials.add(Color::srgba(1.0, 1.0, 1.0, 0.85))),
        Transform::from_xyz(0.0, 0.0, 0.5),
        Visibility::Hidden,
        HexHighlight,
    ));

    // Each noot owns a unique material so `update_noot_color` can tint it alone.
    let mut sim_rng = Rng::new(world.seed ^ 0xA5A5_5A5A);
    let noot_mesh = meshes.add(Circle::new(hex_size * 0.28));

    // Shared actor-critic brain: reuse the saved one if it fits this map, else fresh.
    let policy = match restore_policy {
        Some(p) if p.fits(n_tiles) => p,
        _ => ActorCritic::new(n_tiles, &mut sim_rng),
    };
    commands.insert_resource(policy);

    match restore_noots {
        // Resume: respawn every saved noot with its components and learned field.
        Some(noots) => {
            for ns in noots {
                let (col, row) = (ns.pos.col, ns.pos.row);
                let color = if ns.claim.hex.is_some() {
                    NOOT_OWNER
                } else {
                    NOOT_UNCLAIMED
                };
                spawn_restored_noot(
                    &mut commands,
                    &mut sim_rng,
                    noot_mesh.clone(),
                    materials.add(color),
                    ns,
                    tile_to_pixel(col, row, hex_size, offset),
                );
            }
        }
        // Fresh: everyone spawns claimless and free-roaming; mining is emergent
        // (a noot claims the first unowned deposit it crosses).
        None => {
            for _ in 0..N_NOOTS {
                let (col, row) = random_tile(&mut sim_rng, &world);
                spawn_noot(
                    &mut commands,
                    &mut sim_rng,
                    noot_mesh.clone(),
                    materials.add(NOOT_UNCLAIMED),
                    None,
                    col,
                    row,
                    tile_to_pixel(col, row, hex_size, offset),
                );
            }
        }
    }

    spawn_ui(&mut commands, &ui_font, &graph_assets);

    commands.insert_resource(SimRng(sim_rng));
    commands.insert_resource(Sim(world));
}

/// Respawn a noot from a save: its saved components plus a fresh `PolicyMemory`
/// (transient RL cache) carrying the saved exploration ε.
fn spawn_restored_noot(
    commands: &mut Commands,
    rng: &mut Rng,
    mesh: Handle<Mesh>,
    material: Handle<ColorMaterial>,
    ns: save::NootSave,
    pixel: Vec2,
) {
    // A pre-names save loads unnamed; give those a fresh name on resume.
    let name = if ns.name.is_unnamed() {
        NootName::random(rng)
    } else {
        ns.name
    };
    commands.spawn((
        Mesh2d(mesh),
        MeshMaterial2d(material),
        Transform::from_xyz(pixel.x, pixel.y, 2.0),
        Noot,
        Action::default(),
        ns.claim,
        ns.trader,
        ns.meta,
        name,
        ns.pos,
        ns.inv,
        ns.wallet,
        ns.hunger,
        PolicyMemory::new(ns.explore),
    ));
}

fn random_tile(rng: &mut Rng, world: &World) -> (i32, i32) {
    (
        rng.below(world.cols as usize) as i32,
        rng.below(world.rows as usize) as i32,
    )
}

#[allow(clippy::too_many_arguments)]
fn spawn_noot(
    commands: &mut Commands,
    rng: &mut Rng,
    mesh: Handle<Mesh>,
    material: Handle<ColorMaterial>,
    claim: Option<usize>,
    col: i32,
    row: i32,
    pixel: Vec2,
) {
    commands.spawn((
        Mesh2d(mesh),
        MeshMaterial2d(material),
        Transform::from_xyz(pixel.x, pixel.y, 2.0),
        Noot,
        Action::default(),
        Claim::new(claim),
        Trader::new(),
        NootMeta::new(),
        NootName::random(rng),
        TilePos { col, row },
        Inventory::new(),
        Wallet {
            bucks: STARTING_BUCKS,
        },
        Hunger::fresh(rng),
        PolicyMemory::new(rng.range(EXPLORE_MIN, EXPLORE_MAX)),
    ));
}

fn spawn_ui(commands: &mut Commands, font: &Handle<Font>, graphs: &GraphAssets) {
    commands
        .spawn(Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::FlexEnd,
            padding: UiRect::all(Val::Px(10.0)),
            ..default()
        })
        .with_children(|root| {
            // Selection panel (bottom): details of the followed noot.
            root.spawn((
                Node {
                    align_self: AlignSelf::FlexStart,
                    max_width: Val::Percent(100.0),
                    padding: UiRect::all(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            ))
            .with_children(|panel| {
                panel.spawn((
                    Text::new("tap a noot to follow it, or a hex to inspect it"),
                    TextFont {
                        font: font.clone(),
                        font_size: 13.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    SelectionText,
                ));
            });
        });

    // Graphs panel (hidden until toggled). Spawned before the buttons so the toggle
    // column renders on top of it and stays tappable while the panel is open.
    spawn_graphs_panel(commands, font, graphs);

    // Transport bar, pinned top-right (absolute so it floats over the panels): a row of
    // [<<] [Play/Pause] [>>] with the ticks/s readout. Touch-target-sized buttons.
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            right: Val::Px(PAUSE_BTN_MARGIN),
            top: Val::Px(PAUSE_BTN_MARGIN),
            height: Val::Px(PAUSE_BTN_H),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(XPORT_GAP),
            ..default()
        })
        .with_children(|bar| {
            spawn_transport_button(bar, font, "<<", XPORT_SIDE_W, SpeedDownButton);
            // Middle play/pause button (caption flips in `pause_controls`).
            bar.spawn((
                Button,
                Node {
                    width: Val::Px(XPORT_MID_W),
                    height: Val::Px(PAUSE_BTN_H),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(BTN_OFF),
                PauseButton,
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new("Pause"),
                    TextFont {
                        font: font.clone(),
                        font_size: 15.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    PauseLabel,
                ));
            });
            spawn_transport_button(bar, font, ">>", XPORT_SIDE_W, SpeedUpButton);
            // Ticks/s readout.
            bar.spawn((
                Text::new(speed_label(SPEED_STEPS[SPEED_DEFAULT_IDX])),
                TextFont {
                    font: font.clone(),
                    font_size: 13.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                Node {
                    width: Val::Px(XPORT_READOUT_W),
                    margin: UiRect::left(Val::Px(2.0)),
                    ..default()
                },
                SpeedLabel,
            ));
        });

    // Map-overlay cycle button (caption shows the active mode): off → terrain → trades.
    commands
        .spawn((
            Button,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(PAUSE_BTN_MARGIN),
                top: Val::Px(OVERLAY_BTN_TOP),
                width: Val::Px(PAUSE_BTN_W),
                height: Val::Px(PAUSE_BTN_H),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(BTN_OFF),
            MapOverlayButton,
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(MapOverlayMode::default().label()),
                TextFont {
                    font: font.clone(),
                    font_size: 15.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                MapOverlayLabel,
            ));
        });
    // Save button spawned manually (not via the helper) so its caption carries a
    // `SaveLabel` marker — `save_game` flips it to "Saved!" as save confirmation.
    commands
        .spawn((
            Button,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(PAUSE_BTN_MARGIN),
                top: Val::Px(SAVE_BTN_TOP),
                width: Val::Px(PAUSE_BTN_W),
                height: Val::Px(PAUSE_BTN_H),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(BTN_OFF),
            SaveButton,
        ))
        .with_children(|b| {
            b.spawn((
                Text::new("Save"),
                TextFont {
                    font: font.clone(),
                    font_size: 16.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                SaveLabel,
            ));
        });
    spawn_overlay_button(commands, font, "New", NEW_BTN_TOP, NewWorldButton);
    spawn_overlay_button(commands, font, "Graphs", GRAPHS_BTN_TOP, GraphsButton);
    spawn_overlay_button(commands, font, "Prices", PRICES_BTN_TOP, PricesButton);
    spawn_overlay_button(commands, font, "Wealth", WEALTH_BTN_TOP, WealthButton);

    // Noot-colouring cycle button (caption shows the active mode).
    commands
        .spawn((
            Button,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(PAUSE_BTN_MARGIN),
                top: Val::Px(NOOT_BTN_TOP),
                width: Val::Px(PAUSE_BTN_W),
                height: Val::Px(PAUSE_BTN_H),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(BTN_OFF),
            NootColorButton,
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(NootColorMode::default().label()),
                TextFont {
                    font: font.clone(),
                    font_size: 15.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                NootColorLabel,
            ));
        });
}

/// Spawn one top-right overlay toggle button at vertical offset `top`.
fn spawn_overlay_button(
    commands: &mut Commands,
    font: &Handle<Font>,
    label: &str,
    top: f32,
    marker: impl Component,
) {
    commands
        .spawn((
            Button,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(PAUSE_BTN_MARGIN),
                top: Val::Px(top),
                width: Val::Px(PAUSE_BTN_W),
                height: Val::Px(PAUSE_BTN_H),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(BTN_OFF),
            marker,
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(label),
                TextFont {
                    font: font.clone(),
                    font_size: 16.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });
}

/// Spawn one transport-bar button (a child of the bar row) carrying `label`.
fn spawn_transport_button(
    bar: &mut bevy::ecs::hierarchy::ChildSpawnerCommands,
    font: &Handle<Font>,
    label: &str,
    width: f32,
    marker: impl Component,
) {
    bar.spawn((
        Button,
        Node {
            width: Val::Px(width),
            height: Val::Px(PAUSE_BTN_H),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(BTN_OFF),
        marker,
    ))
    .with_children(|b| {
        b.spawn((
            Text::new(label),
            TextFont {
                font: font.clone(),
                font_size: 18.0,
                ..default()
            },
            TextColor(Color::WHITE),
        ));
    });
}

/// The transport bar's speed readout, e.g. `60.0 ticks/s`.
fn speed_label(tps: f32) -> String {
    format!("{:.1} ticks/s", tps)
}

/// Build the (initially hidden) graphs panel: the big correlation chart on top, then a
/// wrap-grid of tappable per-stat sparkline cells below.
fn spawn_graphs_panel(commands: &mut Commands, font: &Handle<Font>, graphs: &GraphAssets) {
    // --- Top sparkline strip (always docked at the top, collapsible) ---------
    // Spans the width but leaves the right button column clear; the collapse toggle
    // sits on its own row so the strip never fully disappears.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexStart,
                row_gap: Val::Px(3.0),
                padding: UiRect {
                    left: Val::Px(8.0),
                    right: Val::Px(PAUSE_BTN_W + 2.0 * PAUSE_BTN_MARGIN),
                    top: Val::Px(6.0),
                    bottom: Val::Px(6.0),
                },
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.62)),
        ))
        .with_children(|strip| {
            // Collapse/expand toggle (always visible).
            strip
                .spawn((
                    Button,
                    Node {
                        padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)),
                        ..default()
                    },
                    BackgroundColor(BTN_OFF),
                    StripToggle,
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new("Hide stats"),
                        TextFont {
                            font: font.clone(),
                            font_size: 13.0,
                            ..default()
                        },
                        TextColor(Color::WHITE),
                        StripToggleLabel,
                    ));
                });

            // The wrapping row of tappable stat cells (hidden when collapsed).
            strip
                .spawn((
                    Node {
                        width: Val::Percent(100.0),
                        flex_direction: FlexDirection::Row,
                        flex_wrap: FlexWrap::Wrap,
                        align_items: AlignItems::FlexStart,
                        column_gap: Val::Px(5.0),
                        row_gap: Val::Px(5.0),
                        ..default()
                    },
                    StatStripBody,
                ))
                .with_children(|grid| {
                    for (series, (label, _c, _u)) in graph::SERIES.iter().enumerate() {
                        spawn_stat_cell(grid, font, graphs, series, label);
                    }
                });
        });

    // --- Correlation chart panel (on-demand, behind the Graphs button) -------
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(8.0),
                bottom: Val::Px(70.0),
                // Closed by default; `display: none` keeps it out of layout + picking.
                display: Display::None,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexStart,
                row_gap: Val::Px(3.0),
                padding: UiRect::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.85)),
            GraphsPanel,
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new("Correlation — tap stats above to overlay them"),
                TextFont {
                    font: font.clone(),
                    font_size: 11.0,
                    ..default()
                },
                TextColor(Color::srgb(0.8, 0.8, 0.85)),
            ));
            panel.spawn((
                ImageNode::new(graphs.overlay.clone()),
                Node {
                    width: Val::Px(OVERLAY_W as f32),
                    height: Val::Px(OVERLAY_H as f32),
                    ..default()
                },
            ));
        });

    // --- Per-resource price panel (on-demand, behind the Prices button) ------
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(8.0),
                bottom: Val::Px(70.0),
                display: Display::None,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexStart,
                row_gap: Val::Px(3.0),
                padding: UiRect::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.85)),
            PricesPanel,
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new("Resource prices (* = staple food; ₦ last sale, flat while unsold)"),
                TextFont {
                    font: font.clone(),
                    font_size: 11.0,
                    ..default()
                },
                TextColor(Color::srgb(0.8, 0.8, 0.85)),
            ));
            panel
                .spawn((Node {
                    width: Val::Px(OVERLAY_W as f32),
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    align_items: AlignItems::FlexStart,
                    column_gap: Val::Px(5.0),
                    row_gap: Val::Px(5.0),
                    ..default()
                },))
                .with_children(|grid| {
                    for item in 0..goods::N_ITEMS {
                        spawn_price_cell(grid, font, graphs, item);
                    }
                });
        });

    // --- Wealth-distribution panel (on-demand, behind the Wealth button) -----
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(8.0),
                bottom: Val::Px(70.0),
                display: Display::None,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexStart,
                row_gap: Val::Px(3.0),
                padding: UiRect::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.85)),
            WealthPanel,
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new("Money per noot (richest → poorest)"),
                TextFont {
                    font: font.clone(),
                    font_size: 11.0,
                    ..default()
                },
                TextColor(Color::srgb(0.8, 0.8, 0.85)),
                WealthLabel,
            ));
            panel.spawn((
                ImageNode::new(graphs.wealth.clone()),
                Node {
                    width: Val::Px(OVERLAY_W as f32),
                    height: Val::Px(OVERLAY_H as f32),
                    ..default()
                },
            ));
        });
}

/// One per-resource price cell (caption + sparkline) in the Prices panel. Unlike the
/// stat cells these aren't tappable — each resource just gets its own standalone graph.
fn spawn_price_cell(
    grid: &mut bevy::ecs::hierarchy::ChildSpawnerCommands,
    font: &Handle<Font>,
    graphs: &GraphAssets,
    item: usize,
) {
    grid.spawn(Node {
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        padding: UiRect::all(Val::Px(3.0)),
        row_gap: Val::Px(1.0),
        ..default()
    })
    .with_children(|cell| {
        cell.spawn((
            Text::new("…"),
            TextFont {
                font: font.clone(),
                font_size: 10.0,
                ..default()
            },
            TextColor(Color::WHITE),
            PriceLabel { item },
        ));
        cell.spawn((
            ImageNode::new(graphs.price_sparks[item].clone()),
            Node {
                width: Val::Px(SPARK_W as f32),
                height: Val::Px(SPARK_H as f32),
                ..default()
            },
        ));
    });
}

/// One tappable stat cell (caption + sparkline) in the top strip.
fn spawn_stat_cell(
    grid: &mut bevy::ecs::hierarchy::ChildSpawnerCommands,
    font: &Handle<Font>,
    graphs: &GraphAssets,
    series: usize,
    label: &str,
) {
    grid.spawn((
        Button,
        Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            padding: UiRect::all(Val::Px(3.0)),
            row_gap: Val::Px(1.0),
            ..default()
        },
        BackgroundColor(GRAPH_CELL_OFF),
        GraphCell { series },
    ))
    .with_children(|cell| {
        cell.spawn((
            Text::new(label),
            TextFont {
                font: font.clone(),
                font_size: 10.0,
                ..default()
            },
            TextColor(Color::WHITE),
            GraphLabel { series },
        ));
        cell.spawn((
            ImageNode::new(graphs.sparks[series].clone()),
            Node {
                width: Val::Px(SPARK_W as f32),
                height: Val::Px(SPARK_H as f32),
                ..default()
            },
        ));
    });
}

/// Toggle the graphs panel from the Graphs button or the `C` key, keeping the button
/// tint and the panel visibility in sync.
fn graphs_controls(
    keys: Res<ButtonInput<KeyCode>>,
    button: Query<&Interaction, (Changed<Interaction>, With<GraphsButton>)>,
    mut overlays: ResMut<Overlays>,
    mut panel: Query<&mut Node, With<GraphsPanel>>,
    mut bg: Query<&mut BackgroundColor, With<GraphsButton>>,
) {
    if keys.just_pressed(KeyCode::KeyC) || button.iter().any(|i| *i == Interaction::Pressed) {
        overlays.graphs = !overlays.graphs;
        if let Ok(mut node) = panel.single_mut() {
            node.display = if overlays.graphs {
                Display::Flex
            } else {
                Display::None
            };
        }
        if let Ok(mut bg) = bg.single_mut() {
            bg.0 = if overlays.graphs { VALUE_BTN_ON } else { BTN_OFF };
        }
    }
}

/// Collapse/expand the top sparkline strip from its "Hide/Show stats" toggle, syncing
/// the strip body's display and the toggle caption.
fn strip_controls(
    button: Query<&Interaction, (Changed<Interaction>, With<StripToggle>)>,
    mut overlays: ResMut<Overlays>,
    mut body: Query<&mut Node, With<StatStripBody>>,
    mut label: Query<&mut Text, With<StripToggleLabel>>,
) {
    if button.iter().any(|i| *i == Interaction::Pressed) {
        overlays.strip = !overlays.strip;
        if let Ok(mut node) = body.single_mut() {
            node.display = if overlays.strip {
                Display::Flex
            } else {
                Display::None
            };
        }
        if let Ok(mut text) = label.single_mut() {
            text.0 = if overlays.strip { "Hide stats" } else { "Show stats" }.into();
        }
    }
}

/// Sample every graphed stat into the rolling history (throttled, paused-aware).
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn sample_stats(
    time: Res<Time>,
    paused: Res<Paused>,
    sim: Res<Sim>,
    stats: Res<EconStats>,
    hunger: Res<HungerControl>,
    income: Res<economy::IncomeControl>,
    noots: Query<(&Wallet, &Hunger, &Claim, &NootMeta, &TilePos)>,
    mut hist: ResMut<StatHistory>,
    mut phist: ResMut<PriceHistory>,
    mut timer: Local<f32>,
    mut prev_trades: Local<u64>,
) {
    if paused.0 {
        return;
    }
    *timer += time.delta_secs();
    if *timer < GRAPH_SAMPLE_SECS {
        return;
    }
    *timer = 0.0;

    let (cols, rows) = (sim.0.cols, sim.0.rows);
    let mut n = 0u32;
    let (mut bucks, mut appetite, mut age) = (0.0f32, 0.0f32, 0.0f32);
    let (mut starving, mut claimed) = (0u32, 0u32);
    let mut tiles: Vec<(i32, i32)> = Vec::new();
    for (w, h, c, m, tp) in &noots {
        bucks += w.bucks;
        appetite += h.staple.iter().sum::<f32>() / h.staple.len() as f32;
        age += m.age;
        if h.is_starving() {
            starving += 1;
        }
        if c.hex.is_some() {
            claimed += 1;
        }
        tiles.push((tp.col, tp.row));
        n += 1;
    }
    let nf = n.max(1) as f32;
    let nn = hex::mean_nearest_neighbor(&tiles, cols, rows);

    let trades = stats.trades_total;
    let dtrades = trades.saturating_sub(*prev_trades) as f32;
    *prev_trades = trades;

    let mut s = [0.0f32; graph::N_SERIES];
    s[0] = stats.production_rate;
    s[1] = stats.consumption_rate;
    s[2] = stats.merchant_profit_rate;
    s[3] = stats.utility_rate;
    s[4] = dtrades;
    s[5] = bucks / nf;
    s[6] = appetite / nf;
    s[7] = starving as f32;
    s[8] = claimed as f32;
    s[9] = hunger.rate;
    s[10] = hunger.measured_per_tick;
    s[11] = income.rate;
    s[12] = income.measured_inflation * 100.0;
    s[13] = age / nf;
    s[14] = nn;
    s[15] = stats.gdp_rate;

    hist.0.push(s);
    phist.0.push(stats.last_sale_price);
}

/// Re-rasterize the chart textures and refresh the per-stat captions / selection tints
/// while the panel is open (throttled).
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn render_graphs(
    time: Res<Time>,
    overlays: Res<Overlays>,
    hist: Res<StatHistory>,
    selection: Res<GraphSelection>,
    assets: Res<GraphAssets>,
    mut images: ResMut<Assets<Image>>,
    mut labels: Query<(&GraphLabel, &mut Text)>,
    mut cells: Query<(&GraphCell, &mut BackgroundColor)>,
    mut timer: Local<f32>,
) {
    // Nothing visible → skip the work entirely.
    if !overlays.strip && !overlays.graphs {
        return;
    }
    *timer += time.delta_secs();
    if *timer < 0.2 {
        return;
    }
    *timer = 0.0;

    // Pull each series into a chronological f32 vector once.
    let cols: Vec<Vec<f32>> = (0..graph::N_SERIES)
        .map(|i| hist.0.iter().map(|s| s[i]).collect())
        .collect();

    // Strip: redraw each sparkline, refresh its caption (SI-prefixed value) and tint.
    if overlays.strip {
        for (i, col) in cols.iter().enumerate() {
            if let Some(img) = images.get_mut(&assets.sparks[i]) {
                graph::render_sparkline(img, col, graph::SERIES[i].1);
            }
        }
        for (lbl, mut text) in &mut labels {
            let (name, _c, unit) = graph::SERIES[lbl.series];
            let latest = cols[lbl.series].last().copied().unwrap_or(0.0);
            text.0 = format!("{} {}", name, graph::fmt_value(latest, unit));
        }
        for (cell, mut bg) in &mut cells {
            bg.0 = if selection.0[cell.series] {
                GRAPH_CELL_ON
            } else {
                GRAPH_CELL_OFF
            };
        }
    }

    // Correlation chart: overlay the selected series, each independently normalized.
    if overlays.graphs {
        if let Some(img) = images.get_mut(&assets.overlay) {
            let sel: Vec<(&[f32], [u8; 3])> = (0..graph::N_SERIES)
                .filter(|&i| selection.0[i])
                .map(|i| (cols[i].as_slice(), graph::SERIES[i].1))
                .collect();
            graph::render_overlay(img, &sel);
        }
    }
}

/// Re-rasterize the per-resource price sparklines and refresh their captions while the
/// Prices panel is open (throttled). Each resource gets its own auto-scaled graph; the
/// series holds the last sale price through no-trade spells, so it never drops to zero.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn render_prices(
    time: Res<Time>,
    overlays: Res<Overlays>,
    sim: Res<Sim>,
    phist: Res<PriceHistory>,
    assets: Res<GraphAssets>,
    mut images: ResMut<Assets<Image>>,
    mut labels: Query<(&PriceLabel, &mut Text)>,
    mut timer: Local<f32>,
) {
    if !overlays.prices {
        return;
    }
    *timer += time.delta_secs();
    if *timer < 0.2 {
        return;
    }
    *timer = 0.0;

    // Trace colour cues the good's role: green = staple (food), tan = intermediate
    // (needs refining), gold = positional luxury, grey = unused.
    let world = &sim.0;
    for item in 0..goods::N_ITEMS {
        let col: Vec<f32> = phist.0.iter().map(|s| s[item]).collect();
        if let Some(img) = images.get_mut(&assets.price_sparks[item]) {
            graph::render_sparkline(img, &col, role_color(world.goods.role_of(item)));
        }
    }
    for (lbl, mut text) in &mut labels {
        let slot = lbl.item / 2;
        let name = match goods::form_of(lbl.item) {
            GoodForm::Raw => elements::element(world.chosen[slot].id).name,
            GoodForm::Refined => elements::element(world.chosen[slot].id).refined,
        };
        // Tag staples (the foods) so they stand out among the luxuries.
        let tag = match world.goods.role_of(lbl.item) {
            goods::ItemRole::Staple(_) => "*",
            _ => "",
        };
        let latest = phist.0.back().map(|s| s[lbl.item]).unwrap_or(0.0);
        text.0 = format!("{name}{tag} ₦{latest:.1}");
    }
}

/// Sparkline colour for a good's role: green staple, tan intermediate, gold positional.
fn role_color(role: goods::ItemRole) -> [u8; 3] {
    match role {
        goods::ItemRole::Staple(_) => [120, 210, 120],
        goods::ItemRole::Intermediate => [210, 170, 90],
        goods::ItemRole::Positional(_) => [240, 205, 80],
        goods::ItemRole::Junk => [90, 90, 90],
    }
}

/// Toggle the per-resource price panel from the Prices button or the `P` key, keeping the
/// button tint and the panel visibility in sync.
fn prices_controls(
    keys: Res<ButtonInput<KeyCode>>,
    button: Query<&Interaction, (Changed<Interaction>, With<PricesButton>)>,
    mut overlays: ResMut<Overlays>,
    mut panel: Query<&mut Node, With<PricesPanel>>,
    mut bg: Query<&mut BackgroundColor, With<PricesButton>>,
) {
    if keys.just_pressed(KeyCode::KeyP) || button.iter().any(|i| *i == Interaction::Pressed) {
        overlays.prices = !overlays.prices;
        if let Ok(mut node) = panel.single_mut() {
            node.display = if overlays.prices {
                Display::Flex
            } else {
                Display::None
            };
        }
        if let Ok(mut bg) = bg.single_mut() {
            bg.0 = if overlays.prices { TRADES_BTN_ON } else { BTN_OFF };
        }
    }
}

/// Redraw the wealth-distribution chart while the Wealth panel is open (throttled):
/// every noot's bucks sorted richest → poorest as a zero-based bar chart, plus the Gini
/// coefficient in the caption. A steep convex drop / high Gini = lots of inequality.
#[allow(clippy::type_complexity)]
fn render_wealth(
    time: Res<Time>,
    overlays: Res<Overlays>,
    assets: Res<GraphAssets>,
    noots: Query<&Wallet, With<Noot>>,
    mut images: ResMut<Assets<Image>>,
    mut label: Query<&mut Text, With<WealthLabel>>,
    mut timer: Local<f32>,
) {
    if !overlays.wealth {
        return;
    }
    *timer += time.delta_secs();
    if *timer < 0.3 {
        return;
    }
    *timer = 0.0;

    let mut wealth: Vec<f32> = noots.iter().map(|w| w.bucks).collect();
    let gini = economy::gini(&wealth);
    wealth.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    if let Some(img) = images.get_mut(&assets.wealth) {
        graph::render_bars(img, &wealth, [120, 230, 180]);
    }
    if let Ok(mut text) = label.single_mut() {
        text.0 = format!("Money per noot (richest → poorest) — Gini {gini:.2}");
    }
}

/// Toggle the wealth-distribution panel from the Wealth button or the `I` key, keeping
/// the button tint and the panel visibility in sync.
fn wealth_controls(
    keys: Res<ButtonInput<KeyCode>>,
    button: Query<&Interaction, (Changed<Interaction>, With<WealthButton>)>,
    mut overlays: ResMut<Overlays>,
    mut panel: Query<&mut Node, With<WealthPanel>>,
    mut bg: Query<&mut BackgroundColor, With<WealthButton>>,
) {
    if keys.just_pressed(KeyCode::KeyI) || button.iter().any(|i| *i == Interaction::Pressed) {
        overlays.wealth = !overlays.wealth;
        if let Ok(mut node) = panel.single_mut() {
            node.display = if overlays.wealth {
                Display::Flex
            } else {
                Display::None
            };
        }
        if let Ok(mut bg) = bg.single_mut() {
            bg.0 = if overlays.wealth { TERRAIN_BTN_ON } else { BTN_OFF };
        }
    }
}

/// Tapping a stat cell toggles whether it's drawn on the correlation chart.
fn graph_select(
    cells: Query<(&GraphCell, &Interaction), Changed<Interaction>>,
    mut selection: ResMut<GraphSelection>,
) {
    for (cell, interaction) in &cells {
        if *interaction == Interaction::Pressed {
            selection.0[cell.series] = !selection.0[cell.series];
        }
    }
}

/// Step the sim speed through `SPEED_STEPS` (clamped) from the transport bar's `<<` /
/// `>>` buttons (or the `,` / `.` keys), and keep the ticks/s readout in sync.
fn speed_controls(
    keys: Res<ButtonInput<KeyCode>>,
    down: Query<&Interaction, (Changed<Interaction>, With<SpeedDownButton>)>,
    up: Query<&Interaction, (Changed<Interaction>, With<SpeedUpButton>)>,
    mut speed: ResMut<SimSpeed>,
    mut label: Query<&mut Text, With<SpeedLabel>>,
) {
    let dec = keys.just_pressed(KeyCode::Comma) || down.iter().any(|i| *i == Interaction::Pressed);
    let inc = keys.just_pressed(KeyCode::Period) || up.iter().any(|i| *i == Interaction::Pressed);
    if dec || inc {
        let cur = SPEED_STEPS
            .iter()
            .position(|&s| s == speed.ticks_per_second)
            .unwrap_or(SPEED_DEFAULT_IDX);
        let next = if inc {
            (cur + 1).min(SPEED_STEPS.len() - 1)
        } else {
            cur.saturating_sub(1)
        };
        speed.ticks_per_second = SPEED_STEPS[next];
    }
    if let Ok(mut text) = label.single_mut() {
        let want = speed_label(speed.ticks_per_second);
        if text.0 != want {
            text.0 = want;
        }
    }
}

/// Toggle pause from the spacebar or the on-screen button, and keep the button
/// caption in sync. Runs every frame (never gated) so the sim can be unpaused.
fn pause_controls(
    keys: Res<ButtonInput<KeyCode>>,
    mut paused: ResMut<Paused>,
    button: Query<&Interaction, (Changed<Interaction>, With<PauseButton>)>,
    mut label: Query<&mut Text, With<PauseLabel>>,
) {
    let pressed_button = button.iter().any(|i| *i == Interaction::Pressed);
    if keys.just_pressed(KeyCode::Space) || pressed_button {
        paused.0 = !paused.0;
    }
    if let Ok(mut text) = label.single_mut() {
        let want = if paused.0 { "Play" } else { "Pause" };
        if text.0 != want {
            text.0 = want.into();
        }
    }
}

/// Touch: one finger drags the map, two fingers pinch to zoom (and pan). A
/// deliberate drag also releases any follow lock.
fn touch_camera(
    touches: Res<Touches>,
    mut camera: Query<&mut Transform, With<Camera2d>>,
    mut selection: ResMut<Selection>,
) {
    let Ok(mut transform) = camera.single_mut() else {
        return;
    };
    let scale = transform.scale.x;
    let active: Vec<&Touch> = touches.iter().collect();

    match active.as_slice() {
        [finger] => {
            let delta = finger.delta();
            pan(&mut transform, delta, scale);
            if delta.length() > DESELECT_PAN_SLOP {
                selection.0 = None;
            }
        }
        [a, b, ..] => {
            let mid = (a.delta() + b.delta()) * 0.5;
            pan(&mut transform, mid, scale);
            selection.0 = None;

            let current = (a.position() - b.position()).length();
            let previous = (a.previous_position() - b.previous_position()).length();
            if previous > 1.0 && current > 1.0 {
                let zoom = (scale * previous / current).clamp(MIN_ZOOM, MAX_ZOOM);
                transform.scale = Vec3::splat(zoom);
            }
        }
        _ => {}
    }
}

/// Screen-space drag (y-down) → world-space camera move (y-up).
fn pan(transform: &mut Transform, screen_delta: Vec2, scale: f32) {
    transform.translation.x -= screen_delta.x * scale;
    transform.translation.y += screen_delta.y * scale;
}

/// Desktop convenience: WASD / arrows pan, scroll wheel zooms.
fn keyboard_mouse_camera(
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    mut camera: Query<&mut Transform, With<Camera2d>>,
    mut selection: ResMut<Selection>,
) {
    let Ok(mut transform) = camera.single_mut() else {
        return;
    };
    let scale = transform.scale.x;

    let mut dir = Vec2::ZERO;
    if keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp) {
        dir.y += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) || keys.pressed(KeyCode::ArrowDown) {
        dir.y -= 1.0;
    }
    if keys.pressed(KeyCode::KeyA) || keys.pressed(KeyCode::ArrowLeft) {
        dir.x -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight) {
        dir.x += 1.0;
    }
    if dir != Vec2::ZERO {
        let step = dir.normalize() * 600.0 * scale * time.delta_secs();
        transform.translation.x += step.x;
        transform.translation.y += step.y;
        selection.0 = None;
    }

    if scroll.delta.y != 0.0 {
        let factor = if scroll.delta.y > 0.0 { 0.9 } else { 1.1 };
        transform.scale = Vec3::splat((scale * factor).clamp(MIN_ZOOM, MAX_ZOOM));
    }
}

/// Fit the map to the real window for the first `FIT_SETTLE_SECS` after launch
/// (long enough for the wasm canvas to settle into its parent), then hand zoom to
/// the user. Scale fills the tighter screen axis — fit to width or height by
/// whichever needs the smaller zoom — so the looser axis overflows and can be panned.
fn fit_camera_to_screen(
    time: Res<Time>,
    view: Res<MapView>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut camera: Query<&mut Transform, With<Camera2d>>,
    mut elapsed: Local<f32>,
) {
    if *elapsed > FIT_SETTLE_SECS {
        return;
    }
    *elapsed += time.delta_secs();
    let Ok(window) = windows.single() else {
        return;
    };
    let (w, h) = (window.width(), window.height());
    if w < 1.0 || h < 1.0 {
        return;
    }
    let Ok(mut transform) = camera.single_mut() else {
        return;
    };
    let zoom = (view.map_w / w).min(view.map_h / h).clamp(MIN_ZOOM, MAX_ZOOM);
    transform.scale = Vec3::splat(zoom);
}

/// A tap (touch) or left-click that didn't pan selects the nearest noot under
/// the pointer (or a tapped deposit's owner); an empty hit clears the selection.
// A Bevy system pulling several resources/queries inherently has many params.
#[allow(clippy::too_many_arguments)]
fn pick_selection(
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    overlays: Res<Overlays>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    noots: Query<(Entity, &Transform, &Claim), With<Noot>>,
    sim: Res<Sim>,
    view: Res<MapView>,
    mut selection: ResMut<Selection>,
    mut selected_hex: ResMut<SelectedHex>,
) {
    // The graphs panel covers the map; taps there drive the panel, not noot selection.
    if overlays.graphs {
        return;
    }
    let Ok((camera, cam_tf)) = cameras.single() else {
        return;
    };
    let window = windows.single().ok();

    // A click/tap on the top-right controls (the wide transport bar on the top row, or
    // the narrower toggle column below it) must not be read as an empty map hit (which
    // would clear the selection). Skip those two zones.
    let over_buttons = |p: Vec2| {
        window.is_some_and(|w| {
            let right = w.width() - PAUSE_BTN_MARGIN;
            let bar = p.x >= right - XPORT_BAR_W
                && p.x <= right
                && p.y >= PAUSE_BTN_MARGIN
                && p.y <= PAUSE_BTN_MARGIN + PAUSE_BTN_H;
            let column = p.x >= right - PAUSE_BTN_W
                && p.x <= right
                && p.y >= OVERLAY_BTN_TOP
                && p.y <= BTN_COLUMN_BOTTOM;
            bar || column
        })
    };

    // Collect this frame's pick points in screen space.
    let mut points: Vec<Vec2> = Vec::new();
    // Desktop: no mouse-drag panning exists, so any left click is a pick.
    if mouse.just_pressed(MouseButton::Left) {
        if let Some(cursor) = window.and_then(|w| w.cursor_position()) {
            if !over_buttons(cursor) {
                points.push(cursor);
            }
        }
    }
    // Mobile: a tap is a touch that lifted with little movement (a drag pans).
    for touch in touches.iter_just_released() {
        if (touch.position() - touch.start_position()).length() < TAP_SLOP
            && !over_buttons(touch.position())
        {
            points.push(touch.position());
        }
    }
    if points.is_empty() {
        return;
    }

    let pick_r2 = (view.hex_size * 0.6).powi(2);
    let hex_r2 = (view.hex_size * 1.1).powi(2);
    for screen in points {
        let Ok(world_pos) = camera.viewport_to_world_2d(cam_tf, screen) else {
            continue;
        };
        // Prefer the nearest noot under the pointer — tapping a noot follows it.
        let mut best: Option<(Entity, f32)> = None;
        for (e, tf, _) in &noots {
            let d2 = tf.translation.truncate().distance_squared(world_pos);
            if d2 <= pick_r2 && best.is_none_or(|(_, bd)| d2 < bd) {
                best = Some((e, d2));
            }
        }
        if let Some((e, _)) = best {
            selection.0 = Some(e);
            selected_hex.0 = None;
            continue;
        }
        // No noot hit: inspect the tapped hex (nearest tile centre to the pointer).
        let mut nearest: Option<(usize, f32)> = None;
        for (i, t) in sim.0.tiles.iter().enumerate() {
            let c = tile_to_pixel(t.col, t.row, view.hex_size, view.offset);
            let d2 = c.distance_squared(world_pos);
            if d2 <= hex_r2 && nearest.is_none_or(|(_, bd)| d2 < bd) {
                nearest = Some((i, d2));
            }
        }
        selection.0 = None;
        selected_hex.0 = nearest.map(|(i, _)| i);
    }
}

/// S key or the "Save" button: snapshot the full game state (world, controllers,
/// stats, policy, and every noot) to localStorage so a later reload can resume it.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn save_game(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    button: Query<&Interaction, (Changed<Interaction>, With<SaveButton>)>,
    sim: Res<Sim>,
    hunger: Res<HungerControl>,
    income: Res<economy::IncomeControl>,
    stats: Res<EconStats>,
    policy: Res<ActorCritic>,
    stat_history: Res<StatHistory>,
    price_history: Res<PriceHistory>,
    noots: Query<(
        &TilePos,
        &Inventory,
        &Wallet,
        &Hunger,
        &Claim,
        &Trader,
        &NootMeta,
        &NootName,
        &PolicyMemory,
    )>,
    mut flash: Local<f32>,
    mut bg: Query<&mut BackgroundColor, With<SaveButton>>,
    mut label: Query<&mut Text, With<SaveLabel>>,
) {
    if keys.just_pressed(KeyCode::KeyS) || button.iter().any(|i| *i == Interaction::Pressed) {
        let noot_saves = noots
            .iter()
            .map(|(pos, inv, wal, hun, claim, trader, meta, name, mem)| save::NootSave {
                pos: *pos,
                inv: inv.clone(),
                wallet: wal.clone(),
                hunger: hun.clone(),
                claim: claim.clone(),
                trader: trader.clone(),
                meta: meta.clone(),
                name: name.clone(),
                explore: mem.explore,
            })
            .collect();
        save::store(&save::Snapshot {
            version: save::SAVE_VERSION,
            world: sim.0.clone(),
            hunger: hunger.clone(),
            income: income.clone(),
            stats: stats.clone(),
            policy: policy.clone(),
            noots: noot_saves,
            stat_history: stat_history.0.clone(),
            price_history: price_history.0.clone(),
        });
        *flash = SAVE_FLASH_SECS;
    }

    // Drive the "Saved!" confirmation: green tint + caption while the flash runs down.
    if *flash > 0.0 {
        *flash -= time.delta_secs();
        let active = *flash > 0.0;
        if let Ok(mut bg) = bg.single_mut() {
            bg.0 = if active { SAVE_FLASH_COLOR } else { BTN_OFF };
        }
        if let Ok(mut text) = label.single_mut() {
            let want = if active { "Saved!" } else { "Save" };
            if text.0 != want {
                text.0 = want.into();
            }
        }
    }
}

/// G key or the "New" button: clear the saved snapshot and reload, starting a fresh
/// world. (Reloading re-runs setup, which rolls a new random world.)
fn new_world_controls(
    keys: Res<ButtonInput<KeyCode>>,
    button: Query<&Interaction, (Changed<Interaction>, With<NewWorldButton>)>,
) {
    if keys.just_pressed(KeyCode::KeyG) || button.iter().any(|i| *i == Interaction::Pressed) {
        save::clear();
        save::reload_page();
    }
}

/// Drive the inspection controls: V (or the Overlay button) cycles the map heatmap
/// none → terrain → trades, and N (or the Noots button) cycles how noots are coloured.
/// Keeps the hidden hex cells and the button captions/tints in sync.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn overlay_controls(
    keys: Res<ButtonInput<KeyCode>>,
    mut overlays: ResMut<Overlays>,
    mut coloring: ResMut<NootColoring>,
    map_btn: Query<&Interaction, (Changed<Interaction>, With<MapOverlayButton>)>,
    noot_btn: Query<&Interaction, (Changed<Interaction>, With<NootColorButton>)>,
    mut terrain_cells: Query<
        &mut Visibility,
        (
            With<TerrainOverlay>,
            Without<TradeOverlay>,
            Without<RouteOverlay>,
            Without<RoadOverlay>,
        ),
    >,
    mut trade_cells: Query<
        &mut Visibility,
        (
            With<TradeOverlay>,
            Without<TerrainOverlay>,
            Without<RouteOverlay>,
            Without<RoadOverlay>,
        ),
    >,
    mut route_cells: Query<
        &mut Visibility,
        (
            With<RouteOverlay>,
            Without<TerrainOverlay>,
            Without<TradeOverlay>,
            Without<RoadOverlay>,
        ),
    >,
    mut road_cells: Query<
        &mut Visibility,
        (
            With<RoadOverlay>,
            Without<TerrainOverlay>,
            Without<TradeOverlay>,
            Without<RouteOverlay>,
        ),
    >,
    mut map_bg: Query<&mut BackgroundColor, With<MapOverlayButton>>,
    mut map_label: Query<&mut Text, (With<MapOverlayLabel>, Without<NootColorLabel>)>,
    mut noot_label: Query<&mut Text, (With<NootColorLabel>, Without<MapOverlayLabel>)>,
) {
    // Cycling noot colouring is independent of the map overlay.
    if keys.just_pressed(KeyCode::KeyN) || noot_btn.iter().any(|i| *i == Interaction::Pressed) {
        coloring.0 = coloring.0.next();
        if let Ok(mut text) = noot_label.single_mut() {
            text.0 = coloring.0.label().into();
        }
    }

    if !(keys.just_pressed(KeyCode::KeyV) || map_btn.iter().any(|i| *i == Interaction::Pressed)) {
        return;
    }
    overlays.map = overlays.map.next();
    let mode = overlays.map;
    // Only touch the (many) hex cells when the overlay actually changed.
    let vis = |on: bool| {
        if on {
            Visibility::Visible
        } else {
            Visibility::Hidden
        }
    };
    for mut v in &mut terrain_cells {
        *v = vis(mode == MapOverlayMode::Terrain);
    }
    for mut v in &mut trade_cells {
        *v = vis(mode == MapOverlayMode::Trades);
    }
    for mut v in &mut route_cells {
        *v = vis(mode == MapOverlayMode::Routes);
    }
    for mut v in &mut road_cells {
        *v = vis(mode == MapOverlayMode::Roads);
    }
    if let Ok(mut text) = map_label.single_mut() {
        text.0 = mode.label().into();
    }
    if let Ok(mut bg) = map_bg.single_mut() {
        bg.0 = match mode {
            MapOverlayMode::None => BTN_OFF,
            MapOverlayMode::Terrain => TERRAIN_BTN_ON,
            MapOverlayMode::Trades => TRADES_BTN_ON,
            MapOverlayMode::Routes => ROUTES_BTN_ON,
            MapOverlayMode::Roads => ROADS_BTN_ON,
        };
    }
}

/// Tint each noot per the active colouring mode: by ownership (amber claim / green
/// claimless), or by ranking a property across the population and scaling white
/// (low) → blue (high). Rebuilt each frame since the ranking shifts as noots act.
fn update_noot_color(
    coloring: Res<NootColoring>,
    sim: Res<Sim>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    noots: Query<(
        &Claim,
        &Wallet,
        &Inventory,
        &NootMeta,
        &MeshMaterial2d<ColorMaterial>,
    )>,
) {
    if let NootColorMode::Ownership = coloring.0 {
        for (claim, _, _, _, mat) in &noots {
            if let Some(m) = materials.get_mut(&mat.0) {
                m.color = if claim.hex.is_some() {
                    NOOT_OWNER
                } else {
                    NOOT_UNCLAIMED
                };
            }
        }
        return;
    }

    // Gather the ranked property, then min-max scale it across the population.
    let goods = &sim.0.goods;
    let property = |wallet: &Wallet, inv: &Inventory, meta: &NootMeta| -> f32 {
        match coloring.0 {
            NootColorMode::Age => meta.age,
            NootColorMode::Bucks => wallet.bucks,
            NootColorMode::Transactions => meta.transactions as f32,
            NootColorMode::Positional => (0..goods::N_ITEMS)
                .filter(|&i| matches!(goods.role_of(i), goods::ItemRole::Positional(_)))
                .map(|i| inv.items[i])
                .sum(),
            NootColorMode::Ownership => 0.0,
        }
    };
    let mut data: Vec<(Handle<ColorMaterial>, f32)> = Vec::new();
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for (_, wallet, inv, meta, mat) in &noots {
        let v = property(wallet, inv, meta);
        lo = lo.min(v);
        hi = hi.max(v);
        data.push((mat.0.clone(), v));
    }
    let span = (hi - lo).max(1e-3);
    for (handle, v) in data {
        if let Some(m) = materials.get_mut(&handle) {
            m.color = rank_color((v - lo) / span);
        }
    }
}

/// White (low) → blue (high) ramp for the ranked noot-colouring overlays.
fn rank_color(t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color::srgb(1.0 - 0.8 * t, 1.0 - 0.7 * t, 1.0 - 0.05 * t)
}

/// Recolour the trade-density overlay from the cumulative per-hex trade tally
/// (`EconStats::trade_hexes`): brighter gold where more commerce has cleared, so the
/// emergent marketplaces stand out. Sub-linear (sqrt) ramp so a lightly-traded hex is
/// still faintly visible. Throttled — the tally drifts slowly.
fn update_trade_overlay(
    time: Res<Time>,
    overlays: Res<Overlays>,
    stats: Res<EconStats>,
    mut timer: Local<f32>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    cells: Query<(&TradeOverlay, &MeshMaterial2d<ColorMaterial>)>,
) {
    if overlays.map != MapOverlayMode::Trades {
        return;
    }
    *timer += time.delta_secs();
    if *timer < 0.4 {
        return;
    }
    *timer = 0.0;

    let max = stats.trade_hexes.iter().copied().max().unwrap_or(0).max(1) as f32;
    for (cell, mat) in &cells {
        if let Some(m) = materials.get_mut(&mat.0) {
            let c = stats.trade_hexes.get(cell.tile).copied().unwrap_or(0) as f32;
            let v = (c / max).sqrt();
            m.color = Color::srgba(0.98, 0.80, 0.15, v * 0.9);
        }
    }
}

/// Recolour the movement (route) heat cells from `EconStats::traffic_hexes`, normalised to
/// the busiest hex. Throttled and gated on the Routes overlay being active, like
/// `update_trade_overlay`. The `sqrt` gamma lifts the lightly-travelled corridors so the
/// hauling lanes between deposits and markets read, not just the busiest junction.
fn update_route_overlay(
    time: Res<Time>,
    overlays: Res<Overlays>,
    stats: Res<EconStats>,
    mut timer: Local<f32>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    cells: Query<(&RouteOverlay, &MeshMaterial2d<ColorMaterial>)>,
) {
    if overlays.map != MapOverlayMode::Routes {
        return;
    }
    *timer += time.delta_secs();
    if *timer < 0.4 {
        return;
    }
    *timer = 0.0;

    let max = stats.traffic_hexes.iter().copied().max().unwrap_or(0).max(1) as f32;
    for (cell, mat) in &cells {
        if let Some(m) = materials.get_mut(&mat.0) {
            let c = stats.traffic_hexes.get(cell.tile).copied().unwrap_or(0) as f32;
            let v = (c / max).sqrt();
            m.color = Color::srgba(0.25, 0.70, 0.95, v * 0.9);
        }
    }
}

/// Recolour the live road heat cells from the decaying `World::road` field (already in
/// `[0, 1]`). Throttled and gated on the Roads overlay, like the others. The `sqrt` gamma
/// lifts faint paths so a forming basin shows before it saturates.
fn update_road_overlay(
    time: Res<Time>,
    overlays: Res<Overlays>,
    sim: Res<Sim>,
    mut timer: Local<f32>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    cells: Query<(&RoadOverlay, &MeshMaterial2d<ColorMaterial>)>,
) {
    if overlays.map != MapOverlayMode::Roads {
        return;
    }
    *timer += time.delta_secs();
    if *timer < 0.4 {
        return;
    }
    *timer = 0.0;

    let road = &sim.0.road;
    for (cell, mat) in &cells {
        if let Some(m) = materials.get_mut(&mat.0) {
            let level = road.get(cell.tile).copied().unwrap_or(0.0).clamp(0.0, 1.0);
            m.color = Color::srgba(0.80, 0.62, 0.32, level.sqrt() * 0.9);
        }
    }
}

/// Colour a structure marker by its kind: cyan shop, orange refinery.
fn structure_color(kind: econ_sim::world::StructureKind) -> Color {
    match kind {
        econ_sim::world::StructureKind::Shop => Color::srgb(0.30, 0.80, 0.85),
        econ_sim::world::StructureKind::Refinery => Color::srgb(0.95, 0.55, 0.20),
    }
}

/// The emblem mesh for a kind: an upward triangle (shop) or a diamond (refinery).
fn emblem_mesh(kind: econ_sim::world::StructureKind, a: &StructAssets) -> Handle<Mesh> {
    match kind {
        econ_sim::world::StructureKind::Shop => a.shop_emblem.clone(),
        econ_sim::world::StructureKind::Refinery => a.refinery_emblem.clone(),
    }
}

/// Shared meshes/materials for structure markers, built once on first run (sizes depend on
/// `MapView::hex_size`, only known after setup).
struct StructAssets {
    frame: Handle<Mesh>,
    body: Handle<Mesh>,
    shop_emblem: Handle<Mesh>,
    refinery_emblem: Handle<Mesh>,
    frame_mat: Handle<ColorMaterial>,
    emblem_mat: Handle<ColorMaterial>,
}

/// Spawn a chonky full-hex marker for any newly-built structure — a dark frame hex, a
/// kind-coloured body hex filling the tile, and a white kind-shaped emblem — so structures
/// read clearly apart from the small round noots and the disc-and-icon deposits. Then keep
/// every marker in sync with its kind (a build-over can flip a refinery into a shop or
/// vice-versa): recolour the body and swap the emblem shape. Structures are append-only in
/// `World::structures`, so a `Local` high-water mark tracks how many we've drawn.
#[allow(clippy::too_many_arguments)]
fn sync_structure_markers(
    mut commands: Commands,
    sim: Res<Sim>,
    view: Res<MapView>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut spawned: Local<usize>,
    mut assets: Local<Option<StructAssets>>,
    bodies: Query<(&StructureMarker, &MeshMaterial2d<ColorMaterial>)>,
    mut emblems: Query<(&StructureEmblem, &mut Mesh2d)>,
) {
    let hs = view.hex_size;
    let a = assets.get_or_insert_with(|| StructAssets {
        frame: meshes.add(RegularPolygon::new(hs * 0.97, 6)),
        body: meshes.add(RegularPolygon::new(hs * 0.86, 6)),
        shop_emblem: meshes.add(RegularPolygon::new(hs * 0.36, 3)),
        refinery_emblem: meshes.add(RegularPolygon::new(hs * 0.32, 4)),
        frame_mat: materials.add(Color::srgb(0.06, 0.07, 0.09)),
        emblem_mat: materials.add(Color::srgba(0.97, 0.98, 1.0, 0.95)),
    });
    let n = sim.0.structures.len();
    while *spawned < n {
        let s = &sim.0.structures[*spawned];
        let tile = &sim.0.tiles[s.tile];
        let (x, y) = hex::hex_center(tile.col, tile.row, view.hex_size);
        let (px, py) = (x + view.offset.x, y + view.offset.y);
        // Frame (z 0.98) → body (z 1.0) → emblem (z 1.06); all under the noot layer (2.0)
        // so a noot visiting the shop still draws on top.
        commands.spawn((
            Mesh2d(a.frame.clone()),
            MeshMaterial2d(a.frame_mat.clone()),
            Transform::from_xyz(px, py, 0.98),
        ));
        commands.spawn((
            Mesh2d(a.body.clone()),
            MeshMaterial2d(materials.add(structure_color(s.kind))),
            Transform::from_xyz(px, py, 1.0),
            StructureMarker { structure: *spawned },
        ));
        commands.spawn((
            Mesh2d(emblem_mesh(s.kind, a)),
            MeshMaterial2d(a.emblem_mat.clone()),
            Transform::from_xyz(px, py, 1.06),
            StructureEmblem { structure: *spawned },
        ));
        *spawned += 1;
    }
    // Keep bodies/emblems in sync with kinds (cheap — few structures).
    for (marker, mat_handle) in &bodies {
        if let Some(mat) = materials.get_mut(&mat_handle.0) {
            mat.color = structure_color(sim.0.structures[marker.structure].kind);
        }
    }
    for (emblem, mut mesh2d) in &mut emblems {
        let want = emblem_mesh(sim.0.structures[emblem.structure].kind, a);
        if mesh2d.0 != want {
            mesh2d.0 = want;
        }
    }
}

/// Reposition the pooled stack labels to each hex holding ≥2 noots, captioned with the
/// count; park the unused ones hidden. Throttled — stacks shift slowly. Noots overlap on
/// a shared tile, so this is the only way to read how many are really there.
#[allow(clippy::type_complexity)]
fn update_stack_labels(
    time: Res<Time>,
    sim: Res<Sim>,
    view: Res<MapView>,
    noots: Query<&TilePos, With<Noot>>,
    mut labels: Query<(&mut Text2d, &mut Transform, &mut Visibility), With<StackLabel>>,
    mut timer: Local<f32>,
) {
    *timer += time.delta_secs();
    if *timer < 0.15 {
        return;
    }
    *timer = 0.0;

    let cols = sim.0.cols;
    let n = (cols * sim.0.rows) as usize;
    let mut count = vec![0u32; n];
    for pos in &noots {
        let idx = (pos.row * cols + pos.col) as usize;
        if idx < n {
            count[idx] += 1;
        }
    }
    let mut stacks = count.iter().enumerate().filter(|&(_, &c)| c >= 2);
    for (mut text, mut tf, mut vis) in &mut labels {
        if let Some((idx, &c)) = stacks.next() {
            let (col, row) = (idx as i32 % cols, idx as i32 / cols);
            let (x, y) = hex::hex_center(col, row, view.hex_size);
            tf.translation.x = x + view.offset.x;
            tf.translation.y = y + view.offset.y;
            text.0 = c.to_string();
            *vis = Visibility::Visible;
        } else {
            *vis = Visibility::Hidden;
        }
    }
}

/// Show a deposit's outline ring iff some noot currently claims it.
fn update_deposit_outlines(
    sim: Res<Sim>,
    claims: Query<&Claim, With<Noot>>,
    mut outlines: Query<(&DepositOutline, &mut Visibility)>,
) {
    // A deposit is claimed iff some noot's owned hex is that deposit's tile.
    let mut owned = vec![false; sim.0.deposits.len()];
    for c in &claims {
        if let Some(h) = c.hex {
            if let Some(d) = sim.0.tiles[h].deposit {
                owned[d] = true;
            }
        }
    }
    for (o, mut vis) in &mut outlines {
        *vis = if owned[o.deposit] {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

/// Keep the camera centred on the selected noot (a manual pan clears it).
fn follow_selected(
    selection: Res<Selection>,
    noots: Query<&Transform, (With<Noot>, Without<Camera2d>)>,
    mut camera: Query<&mut Transform, With<Camera2d>>,
) {
    let Some(e) = selection.0 else {
        return;
    };
    let Ok(noot_tf) = noots.get(e) else {
        return;
    };
    let Ok(mut cam) = camera.single_mut() else {
        return;
    };
    let target = noot_tf.translation.truncate();
    let t = 0.15;
    cam.translation.x += (target.x - cam.translation.x) * t;
    cam.translation.y += (target.y - cam.translation.y) * t;
}

/// Show/hide and reposition the highlight ring on the selected noot.
fn update_selection_ring(
    selection: Res<Selection>,
    noots: Query<&Transform, (With<Noot>, Without<SelectionRing>)>,
    mut ring: Query<(&mut Transform, &mut Visibility), With<SelectionRing>>,
) {
    let Ok((mut ring_tf, mut visibility)) = ring.single_mut() else {
        return;
    };
    match selection.0.and_then(|e| noots.get(e).ok()) {
        Some(noot_tf) => {
            ring_tf.translation = noot_tf.translation.truncate().extend(2.5);
            *visibility = Visibility::Visible;
        }
        None => *visibility = Visibility::Hidden,
    }
}

/// Show/hide and reposition the ring highlighting the inspected hex.
fn update_hex_highlight(
    selected: Res<SelectedHex>,
    sim: Res<Sim>,
    view: Res<MapView>,
    mut highlight: Query<(&mut Transform, &mut Visibility), With<HexHighlight>>,
) {
    let Ok((mut tf, mut visibility)) = highlight.single_mut() else {
        return;
    };
    match selected.0 {
        Some(tile) => {
            let t = &sim.0.tiles[tile];
            tf.translation = tile_to_pixel(t.col, t.row, view.hex_size, view.offset).extend(0.5);
            *visibility = Visibility::Visible;
        }
        None => *visibility = Visibility::Hidden,
    }
}

/// One-word gloss of an item's economic role, for the hex inspector.
fn role_word(role: goods::ItemRole) -> &'static str {
    match role {
        goods::ItemRole::Staple(_) => "food",
        goods::ItemRole::Intermediate => "needs refining",
        goods::ItemRole::Positional(_) => "luxury",
        goods::ItemRole::Junk => "unused",
    }
}

/// Human-readable description of a map tile: its terrain, and whatever is on it (a
/// deposit and what it produces, a shop/refinery and its ownership, or open ground).
fn describe_hex(world: &econ_sim::world::World, tile: usize, claimed: bool) -> String {
    use econ_sim::world::{DepositKind, StructureKind};
    let t = &world.tiles[tile];
    let speed = (econ_sim::world::terrain_factor(t.difficulty) * 100.0).round() as i32;
    let terrain = if t.difficulty < 0.33 {
        "open ground"
    } else if t.difficulty < 0.66 {
        "rough ground"
    } else {
        "cliffs"
    };
    let mut s = format!("[hex] col {} row {} — {terrain}, work {speed}%\n", t.col, t.row);
    if let Some(d) = t.deposit {
        let dep = &world.deposits[d];
        let slot = dep.element_slot;
        let el = elements::element(world.chosen[slot].id);
        let stock = match &dep.kind {
            DepositKind::Replenishable { stock, capacity, .. } => {
                format!("replenishable, {stock:.0}/{capacity:.0}")
            }
            DepositKind::Finite { remaining, initial } => {
                format!("finite, {remaining:.0}/{initial:.0} left")
            }
        };
        let raw_role = role_word(world.goods.role_of(goods::item_index(slot, GoodForm::Raw)));
        let ref_role = role_word(world.goods.role_of(goods::item_index(slot, GoodForm::Refined)));
        let who = if claimed {
            "being mined"
        } else {
            "unclaimed — free to mine"
        };
        s.push_str(&format!(
            "{} deposit · {stock} · produces {} ({raw_role}) → {} ({ref_role}) · {who}",
            el.name, el.name, el.refined,
        ));
    } else if let Some(kind) = world.structure_kind(tile) {
        let (what, note) = match kind {
            StructureKind::Shop => ("shop", "a sell waypoint — trade clears here"),
            StructureKind::Refinery => ("refinery", "noots refine intermediates here"),
        };
        let who = if claimed {
            "owned"
        } else {
            "abandoned — free to claim or build over"
        };
        s.push_str(&format!("{what} · {who} · {note}"));
    } else {
        s.push_str("empty ground · a noot could build a shop or refinery here");
    }
    s
}

/// Fill the bottom panel with the selected noot's details.
#[allow(clippy::type_complexity)]
fn update_selection_panel(
    selection: Res<Selection>,
    selected_hex: Res<SelectedHex>,
    sim: Res<Sim>,
    noots: Query<(
        &Claim,
        &Trader,
        &Wallet,
        &Hunger,
        &Inventory,
        &PolicyMemory,
        &NootMeta,
        &Action,
        &NootName,
    )>,
    mut panel: Query<&mut Text, With<SelectionText>>,
) {
    let Ok(mut text) = panel.single_mut() else {
        return;
    };
    let hint = "tap a noot to follow it, or a hex to inspect it";
    // No noot followed: show the inspected hex, or the hint.
    let Some(entity) = selection.0 else {
        text.0 = match selected_hex.0 {
            Some(tile) => {
                let claimed = noots.iter().any(|(c, ..)| c.hex == Some(tile));
                describe_hex(&sim.0, tile, claimed)
            }
            None => hint.into(),
        };
        return;
    };
    let Ok((claim, trader, wallet, hunger, inv, mem, meta, action, name)) = noots.get(entity) else {
        text.0 = hint.into();
        return;
    };

    let world = &sim.0;
    // The one hex this noot owns, classified by its improvement.
    let claim_label = match claim.hex {
        Some(h) => {
            if let Some(d) = world.tiles[h].deposit {
                let slot = world.deposits[d].element_slot;
                format!("mining {}", elements::element(world.chosen[slot].id).name)
            } else {
                match world.structure_kind(h) {
                    Some(econ_sim::world::StructureKind::Shop) => "owns shop".to_string(),
                    Some(econ_sim::world::StructureKind::Refinery) => "owns refinery".to_string(),
                    None => "unclaimed".to_string(),
                }
            }
        }
        None => "unclaimed".to_string(),
    };
    let act = match action {
        Action::Move => "move",
        Action::Mine => "mine",
        Action::Refine => "refine",
        Action::Idle => "idle",
        Action::BuildShop => "build shop",
        Action::BuildRefinery => "build refinery",
    };

    let utility = economy::maslow_utility(hunger, inv, wallet, &world.goods);
    let mut out = format!(
        "[selected] {} — {}   action {}   skill {:.2}×   discount {:.2}   explore {:.2}   ₦{:.0}   hunger {:.1}   utility {:.2}\n",
        name.display(),
        claim_label,
        act,
        economy::skill_factor(meta.experience),
        trader.discount,
        mem.explore,
        wallet.bucks,
        hunger.staple.iter().sum::<f32>() / hunger.staple.len() as f32,
        utility,
    );

    // Held goods (non-trivial quantities).
    let mut held = String::new();
    for item in 0..goods::N_ITEMS {
        if inv.items[item] > 0.05 {
            let slot = item / 2;
            let name = match goods::form_of(item) {
                GoodForm::Raw => elements::element(world.chosen[slot].id).name,
                GoodForm::Refined => elements::element(world.chosen[slot].id).refined,
            };
            held.push_str(&format!("{} {:.1}  ", name, inv.items[item]));
        }
    }
    if held.is_empty() {
        held.push_str("(nothing)");
    }
    out.push_str(&format!("holding: {}\n", held));
    text.0 = out;
}
