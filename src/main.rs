//! GUI binary: the rendered, interactive app. The simulation core lives in the
//! `econ_sim` library; this file is the rendering/UI/input layer (gated by `gui`).

use bevy::ecs::schedule::ScheduleLabel;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::input::touch::Touch;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use econ_sim::economy::{self, EconStats, HungerControl};
use econ_sim::goods::{self, GoodCategory, GoodForm};
use econ_sim::movement::{self, tile_to_pixel};
use econ_sim::noot::{
    Action, Claim, Hunger, Inventory, Noot, NootMeta, TilePos, Trader, Wallet, EXPLORE_MAX,
    EXPLORE_MIN, STARTING_BUCKS,
};
use econ_sim::policy::{ActorCritic, PolicyMemory, Trainer};
use econ_sim::world::{generate, ResourceRole, World};
use econ_sim::{elements, hex, icon, rng::Rng, save, MapView, Sim, SimRng};

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
// Pause sits at the top; the two overlay toggles stack below it.
const PAUSE_BTN_W: f32 = 120.0;
const PAUSE_BTN_H: f32 = 44.0;
const PAUSE_BTN_MARGIN: f32 = 10.0;
const BTN_GAP: f32 = 8.0;
const VALUE_BTN_TOP: f32 = PAUSE_BTN_MARGIN + PAUSE_BTN_H + BTN_GAP;
const TERRAIN_BTN_TOP: f32 = VALUE_BTN_TOP + PAUSE_BTN_H + BTN_GAP;
const NOOT_BTN_TOP: f32 = TERRAIN_BTN_TOP + PAUSE_BTN_H + BTN_GAP;
const SAVE_BTN_TOP: f32 = NOOT_BTN_TOP + PAUSE_BTN_H + BTN_GAP;
const NEW_BTN_TOP: f32 = SAVE_BTN_TOP + PAUSE_BTN_H + BTN_GAP;
const SPEED_BTN_TOP: f32 = NEW_BTN_TOP + PAUSE_BTN_H + BTN_GAP;
/// Bottom edge of the whole button column (taps above this are UI, not the map).
const BTN_COLUMN_BOTTOM: f32 = SPEED_BTN_TOP + PAUSE_BTN_H;

const BTN_OFF: Color = Color::srgba(0.12, 0.12, 0.12, 0.85);
const VALUE_BTN_ON: Color = Color::srgba(0.62, 0.22, 0.16, 0.9);
const TERRAIN_BTN_ON: Color = Color::srgba(0.20, 0.45, 0.30, 0.9);

/// How long after launch the camera keeps re-fitting the map, so the wasm canvas
/// (which resizes to its parent post-launch) settles before zoom is left to the user.
const FIT_SETTLE_SECS: f32 = 0.3;

/// The noot currently selected and followed, if any.
#[derive(Resource, Default)]
struct Selection(Option<Entity>);

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

/// Selectable game speeds in **ticks per second**, cycled by the Speed button.
/// 60 t/s ≈ real-time; higher values run the sim faster than the wall clock while
/// the render loop stays at the browser's frame rate.
const SPEED_STEPS: [f32; 5] = [60.0, 120.0, 240.0, 480.0, 960.0];

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
            ticks_per_second: SPEED_STEPS[0],
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

/// On-screen overlay toggles (touch equivalents of the V / T keys).
#[derive(Component)]
struct ValueButton;
#[derive(Component)]
struct TerrainButton;
/// Cycles the noot-colouring mode (touch equivalent of the N key).
#[derive(Component)]
struct NootColorButton;
/// Clears the save and rerolls a fresh world (touch equivalent of the G key).
#[derive(Component)]
struct NewWorldButton;
/// Snapshots the full game state to localStorage (touch equivalent of the S key).
#[derive(Component)]
struct SaveButton;
/// Cycles the simulation speed (ticks per second).
#[derive(Component)]
struct SpeedButton;
/// Caption on the speed button, kept in sync with the active speed.
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

#[derive(Component)]
struct HudText;

/// One per-resource HUD row's text (paired with its element icon); `slot` is 0..4.
#[derive(Component)]
struct ResourceLine {
    slot: usize,
}

/// Text of the bottom panel describing the selected noot.
#[derive(Component)]
struct SelectionText;

/// The highlight ring drawn around the selected noot.
#[derive(Component)]
struct SelectionRing;

/// Which inspection overlays are currently shown (toggled with V / T).
#[derive(Resource, Default)]
struct Overlays {
    value: bool,
    terrain: bool,
}

/// Per-hex heat cell for the aggregated noot value-field overlay (`tile` indexes
/// `RouteMemory::value`, i.e. `row * cols + col`).
#[derive(Component)]
struct ValueOverlay {
    tile: usize,
}

/// Per-hex tint cell for the terrain-difficulty overlay (static colour).
#[derive(Component)]
struct TerrainOverlay;

/// Ring drawn around a deposit while it is claimed.
#[derive(Component)]
struct DepositOutline {
    deposit: usize,
}

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
        .init_resource::<Paused>()
        .init_resource::<SimSpeed>()
        .init_resource::<Overlays>()
        .init_resource::<NootColoring>()
        .init_resource::<economy::IncomeControl>()
        .init_resource::<Trainer>()
        .add_systems(Startup, setup)
        // The simulation runs on a fixed-tick schedule (each system advances the
        // world by one `TICK_DT`), driven many times per frame by `run_sim_ticks`.
        // Order matches the headless harness; chained so reads see prior writes.
        .add_systems(
            SimSchedule,
            (
                economy::simulate,
                economy::income,
                economy::income_controller,
                economy::hunger_tick,
                economy::hunger_pid,
                economy::age_noots,
                economy::policy_step,
                economy::claim_deposits,
                economy::extract,
                economy::refine,
                economy::meet_and_trade,
                economy::consume,
                economy::death_and_respawn,
                economy::update_rates,
                economy::train_policy,
            )
                .chain(),
        )
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
                    fit_camera_to_screen,
                ),
                (
                    update_hud,
                    update_selection_ring,
                    update_selection_panel,
                    update_noot_color,
                    update_value_overlay,
                    update_deposit_outlines,
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

    // Tiles share one neutral material — difficulty is shown *only* via the
    // toggleable terrain overlay, so it never fights the value heat overlay. Each
    // tile also gets two hidden overlay cells stacked above it (z 0.4 terrain,
    // z 1.6 value) — the value heat sits just under the noot layer (z 2.0).
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

        // Value-field heat overlay: recoloured each tick by `update_value_overlay`.
        // Born translucent (alpha < 1) so the material is created in blend mode.
        let idx = (tile.row * world.cols + tile.col) as usize;
        commands.spawn((
            Mesh2d(hex_mesh.clone()),
            MeshMaterial2d(materials.add(Color::srgba(1.0, 0.12, 0.04, 0.0))),
            Transform::from_xyz(px, py, 1.6),
            Visibility::Hidden,
            ValueOverlay { tile: idx },
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
                let color = if ns.claim.deposit.is_some() {
                    NOOT_OWNER
                } else {
                    NOOT_UNCLAIMED
                };
                spawn_restored_noot(
                    &mut commands,
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

    spawn_ui(&mut commands, &icons, &ui_font);

    commands.insert_resource(SimRng(sim_rng));
    commands.insert_resource(Sim(world));
}

/// Respawn a noot from a save: its saved components plus a fresh `PolicyMemory`
/// (transient RL cache) carrying the saved exploration ε.
fn spawn_restored_noot(
    commands: &mut Commands,
    mesh: Handle<Mesh>,
    material: Handle<ColorMaterial>,
    ns: save::NootSave,
    pixel: Vec2,
) {
    commands.spawn((
        Mesh2d(mesh),
        MeshMaterial2d(material),
        Transform::from_xyz(pixel.x, pixel.y, 2.0),
        Noot,
        Action::default(),
        ns.claim,
        ns.trader,
        ns.meta,
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
        TilePos { col, row },
        Inventory::new(),
        Wallet {
            bucks: STARTING_BUCKS,
        },
        Hunger::fresh(rng),
        PolicyMemory::new(rng.range(EXPLORE_MIN, EXPLORE_MAX)),
    ));
}

fn spawn_ui(commands: &mut Commands, icons: &[Handle<Image>; 4], font: &Handle<Font>) {
    commands
        .spawn(Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::SpaceBetween,
            padding: UiRect::all(Val::Px(10.0)),
            ..default()
        })
        .with_children(|root| {
            // Status panel (top): summary text, then one icon+text row per resource.
            root.spawn((
                Node {
                    align_self: AlignSelf::FlexStart,
                    max_width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(2.0),
                    padding: UiRect::all(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            ))
            .with_children(|panel| {
                panel.spawn((
                    Text::new("loading..."),
                    TextFont {
                        font: font.clone(),
                        font_size: 14.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    HudText,
                ));
                for (slot, icon) in icons.iter().enumerate() {
                    panel
                        .spawn(Node {
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(6.0),
                            ..default()
                        })
                        .with_children(|row| {
                            row.spawn((
                                ImageNode::new(icon.clone()),
                                Node {
                                    width: Val::Px(18.0),
                                    height: Val::Px(18.0),
                                    ..default()
                                },
                            ));
                            row.spawn((
                                Text::new(""),
                                TextFont {
                                    font: font.clone(),
                                    font_size: 13.0,
                                    ..default()
                                },
                                TextColor(Color::WHITE),
                                ResourceLine { slot },
                            ));
                        });
                }
            });

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
                    Text::new("tap a noot to follow it"),
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

    // Pause toggle, pinned top-right (absolute so it floats over the panels).
    // Large enough to be a comfortable touch target on mobile.
    commands
        .spawn((
            Button,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(PAUSE_BTN_MARGIN),
                top: Val::Px(PAUSE_BTN_MARGIN),
                width: Val::Px(PAUSE_BTN_W),
                height: Val::Px(PAUSE_BTN_H),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.12, 0.12, 0.12, 0.85)),
            PauseButton,
        ))
        .with_children(|b| {
            b.spawn((
                Text::new("Pause"),
                TextFont {
                    font: font.clone(),
                    font_size: 18.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                PauseLabel,
            ));
        });

    // Overlay toggles, stacked under the pause button (same touch-target size).
    spawn_overlay_button(commands, font, "Value", VALUE_BTN_TOP, ValueButton);
    spawn_overlay_button(commands, font, "Terrain", TERRAIN_BTN_TOP, TerrainButton);
    spawn_overlay_button(commands, font, "Save", SAVE_BTN_TOP, SaveButton);
    spawn_overlay_button(commands, font, "New", NEW_BTN_TOP, NewWorldButton);

    // Speed cycle button (caption shows the active multiplier).
    commands
        .spawn((
            Button,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(PAUSE_BTN_MARGIN),
                top: Val::Px(SPEED_BTN_TOP),
                width: Val::Px(PAUSE_BTN_W),
                height: Val::Px(PAUSE_BTN_H),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(BTN_OFF),
            SpeedButton,
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(speed_label(SPEED_STEPS[0])),
                TextFont {
                    font: font.clone(),
                    font_size: 15.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                SpeedLabel,
            ));
        });

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

/// Caption for the speed button: the tick rate as a multiple of the 60 t/s base.
fn speed_label(tps: f32) -> String {
    format!("Speed {:.0}\u{00d7}", tps / SPEED_STEPS[0])
}

/// Cycle the sim speed from the on-screen button, advancing to the next entry in
/// `SPEED_STEPS` (wrapping), and keep the caption in sync.
fn speed_controls(
    button: Query<&Interaction, (Changed<Interaction>, With<SpeedButton>)>,
    mut speed: ResMut<SimSpeed>,
    mut label: Query<&mut Text, With<SpeedLabel>>,
) {
    if button.iter().any(|i| *i == Interaction::Pressed) {
        let cur = SPEED_STEPS
            .iter()
            .position(|&s| s == speed.ticks_per_second)
            .unwrap_or(0);
        speed.ticks_per_second = SPEED_STEPS[(cur + 1) % SPEED_STEPS.len()];
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
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    noots: Query<(Entity, &Transform, &Claim), With<Noot>>,
    sim: Res<Sim>,
    view: Res<MapView>,
    mut selection: ResMut<Selection>,
) {
    let Ok((camera, cam_tf)) = cameras.single() else {
        return;
    };
    let window = windows.single().ok();

    // A click/tap on the top-right button column (pause + overlay toggles) must not
    // be read as an empty map hit (which would clear the selection). Skip those.
    let over_buttons = |p: Vec2| {
        window.is_some_and(|w| {
            let left = w.width() - PAUSE_BTN_MARGIN - PAUSE_BTN_W;
            p.x >= left
                && p.x <= w.width() - PAUSE_BTN_MARGIN
                && p.y >= PAUSE_BTN_MARGIN
                && p.y <= BTN_COLUMN_BOTTOM
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
    let dep_r2 = (view.hex_size * 0.5).powi(2);
    for screen in points {
        let Ok(world_pos) = camera.viewport_to_world_2d(cam_tf, screen) else {
            continue;
        };
        // Prefer the nearest noot under the pointer.
        let mut best: Option<(Entity, f32)> = None;
        for (e, tf, _) in &noots {
            let d2 = tf.translation.truncate().distance_squared(world_pos);
            if d2 <= pick_r2 && best.is_none_or(|(_, bd)| d2 < bd) {
                best = Some((e, d2));
            }
        }
        if let Some((e, _)) = best {
            selection.0 = Some(e);
            continue;
        }
        // No noot hit: if a deposit was tapped, select (and follow) its owner.
        let owner = sim.0.deposits.iter().enumerate().find_map(|(di, dep)| {
            let t = &sim.0.tiles[dep.tile];
            let c = tile_to_pixel(t.col, t.row, view.hex_size, view.offset);
            (c.distance_squared(world_pos) <= dep_r2).then_some(di)
        });
        selection.0 = owner.and_then(|di| {
            noots
                .iter()
                .find(|(_, _, claim)| claim.deposit == Some(di))
                .map(|(e, _, _)| e)
        });
    }
}

/// S key or the "Save" button: snapshot the full game state (world, controllers,
/// stats, policy, and every noot) to localStorage so a later reload can resume it.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn save_game(
    keys: Res<ButtonInput<KeyCode>>,
    button: Query<&Interaction, (Changed<Interaction>, With<SaveButton>)>,
    sim: Res<Sim>,
    hunger: Res<HungerControl>,
    income: Res<economy::IncomeControl>,
    stats: Res<EconStats>,
    policy: Res<ActorCritic>,
    noots: Query<(
        &TilePos,
        &Inventory,
        &Wallet,
        &Hunger,
        &Claim,
        &Trader,
        &NootMeta,
        &PolicyMemory,
    )>,
) {
    if !(keys.just_pressed(KeyCode::KeyS) || button.iter().any(|i| *i == Interaction::Pressed)) {
        return;
    }
    let noot_saves = noots
        .iter()
        .map(|(pos, inv, wal, hun, claim, trader, meta, mem)| save::NootSave {
            pos: *pos,
            inv: inv.clone(),
            wallet: wal.clone(),
            hunger: hun.clone(),
            claim: claim.clone(),
            trader: trader.clone(),
            meta: meta.clone(),
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
    });
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

/// Drive the inspection controls: V/Value toggles the value heat overlay, T/Terrain
/// the difficulty overlay, and N/Noots cycles how noots are coloured. Keeps the
/// hidden hex cells and the button captions/tints in sync.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn overlay_controls(
    keys: Res<ButtonInput<KeyCode>>,
    mut overlays: ResMut<Overlays>,
    mut coloring: ResMut<NootColoring>,
    value_btn: Query<&Interaction, (Changed<Interaction>, With<ValueButton>)>,
    terrain_btn: Query<&Interaction, (Changed<Interaction>, With<TerrainButton>)>,
    noot_btn: Query<&Interaction, (Changed<Interaction>, With<NootColorButton>)>,
    mut value_cells: Query<&mut Visibility, (With<ValueOverlay>, Without<TerrainOverlay>)>,
    mut terrain_cells: Query<&mut Visibility, (With<TerrainOverlay>, Without<ValueOverlay>)>,
    mut value_bg: Query<&mut BackgroundColor, (With<ValueButton>, Without<TerrainButton>)>,
    mut terrain_bg: Query<&mut BackgroundColor, (With<TerrainButton>, Without<ValueButton>)>,
    mut noot_label: Query<&mut Text, With<NootColorLabel>>,
) {
    // Cycling noot colouring is independent of the hex overlays.
    if keys.just_pressed(KeyCode::KeyN) || noot_btn.iter().any(|i| *i == Interaction::Pressed) {
        coloring.0 = coloring.0.next();
        if let Ok(mut text) = noot_label.single_mut() {
            text.0 = coloring.0.label().into();
        }
    }

    let mut changed = false;
    if keys.just_pressed(KeyCode::KeyV) || value_btn.iter().any(|i| *i == Interaction::Pressed) {
        overlays.value = !overlays.value;
        changed = true;
    }
    if keys.just_pressed(KeyCode::KeyT) || terrain_btn.iter().any(|i| *i == Interaction::Pressed) {
        overlays.terrain = !overlays.terrain;
        changed = true;
    }
    if !changed {
        return;
    }
    // Only touch the (many) hex cells and button tints when a toggle flipped.
    let to = |on: bool| {
        if on {
            Visibility::Visible
        } else {
            Visibility::Hidden
        }
    };
    for mut v in &mut value_cells {
        *v = to(overlays.value);
    }
    for mut v in &mut terrain_cells {
        *v = to(overlays.terrain);
    }
    for mut bg in &mut value_bg {
        bg.0 = if overlays.value { VALUE_BTN_ON } else { BTN_OFF };
    }
    for mut bg in &mut terrain_bg {
        bg.0 = if overlays.terrain { TERRAIN_BTN_ON } else { BTN_OFF };
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
                m.color = if claim.deposit.is_some() {
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

/// Recolour the value-heat cells from the shared critic's value over each tile
/// (red = most valued), normalized across the map. Uses a neutral, hungry-and-broke
/// feature snapshot so the surface highlights where the policy expects to do well.
/// Throttled — the net drifts slowly and recolouring every cell each frame is wasteful.
fn update_value_overlay(
    time: Res<Time>,
    overlays: Res<Overlays>,
    sim: Res<Sim>,
    policy: Res<ActorCritic>,
    mut timer: Local<f32>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    cells: Query<(&ValueOverlay, &MeshMaterial2d<ColorMaterial>)>,
) {
    if !overlays.value || policy.n_tiles == 0 {
        return;
    }
    *timer += time.delta_secs();
    if *timer < 0.2 {
        return;
    }
    *timer = 0.0;

    let n = (sim.0.cols * sim.0.rows) as usize;
    // Neutral probe: hungry, broke, no claim, bias; deposit-direction and on-minable
    // features left at zero (the overlay shows the position value surface alone).
    let mut probe = [0.0f32; econ_sim::policy::N_OTHER];
    probe[1] = 1.0;
    probe[2] = 1.0;
    probe[5] = 1.0;
    let mut agg = vec![0.0f32; n];
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for (t, a) in agg.iter_mut().enumerate() {
        let v = policy.value(t, &probe);
        *a = v;
        lo = lo.min(v);
        hi = hi.max(v);
    }
    let span = (hi - lo).max(1e-3);
    for (cell, mat) in &cells {
        if let Some(m) = materials.get_mut(&mat.0) {
            let v = ((agg[cell.tile] - lo) / span).clamp(0.0, 1.0);
            m.color = Color::srgba(1.0, 0.12, 0.04, v * 0.85);
        }
    }
}

/// Show a deposit's outline ring iff some noot currently claims it.
fn update_deposit_outlines(
    sim: Res<Sim>,
    claims: Query<&Claim, With<Noot>>,
    mut outlines: Query<(&DepositOutline, &mut Visibility)>,
) {
    let mut owned = vec![false; sim.0.deposits.len()];
    for c in &claims {
        if let Some(d) = c.deposit {
            owned[d] = true;
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

/// Fill the bottom panel with the selected noot's details.
#[allow(clippy::type_complexity)]
fn update_selection_panel(
    selection: Res<Selection>,
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
    )>,
    mut panel: Query<&mut Text, With<SelectionText>>,
) {
    let Ok(mut text) = panel.single_mut() else {
        return;
    };
    let stale = "tap a noot to follow it";
    let Some(entity) = selection.0 else {
        text.0 = stale.into();
        return;
    };
    let Ok((claim, trader, wallet, hunger, inv, mem, meta, action)) = noots.get(entity) else {
        text.0 = stale.into();
        return;
    };

    let world = &sim.0;
    let claim_label = match claim.deposit {
        Some(d) => {
            let slot = world.deposits[d].element_slot;
            format!("mining {}", elements::element(world.chosen[slot].id).name)
        }
        None => "unclaimed".to_string(),
    };
    let act = match action {
        Action::Move => "move",
        Action::Mine => "mine",
        Action::Refine => "refine",
    };

    let utility = economy::maslow_utility(hunger, inv, wallet, &world.goods);
    let mut out = format!(
        "[selected] noot — {}   action {}   skill {:.2}×   discount {:.2}   explore {:.2}   ₦{:.0}   hunger {:.1}   utility {:.2}\n",
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

#[allow(clippy::too_many_arguments)]
fn update_hud(
    sim: Res<Sim>,
    stats: Res<EconStats>,
    paused: Res<Paused>,
    hunger_ctrl: Res<HungerControl>,
    income_ctrl: Res<economy::IncomeControl>,
    noots: Query<(&Wallet, &Hunger, &Trader, &Claim)>,
    mut hud: Query<&mut Text, (With<HudText>, Without<ResourceLine>)>,
    mut lines: Query<(&ResourceLine, &mut Text), Without<HudText>>,
) {
    let world = &sim.0;

    // Aggregate noot stats over the now-uniform population.
    let mut total_bucks = 0.0f32;
    let mut appetite_sum = 0.0f32;
    let mut discount_sum = 0.0f32;
    let mut starving = 0u32;
    let mut claimed = 0u32;
    let mut count = 0u32;
    for (wallet, hunger, trader, claim) in &noots {
        total_bucks += wallet.bucks;
        appetite_sum += hunger.staple.iter().sum::<f32>() / hunger.staple.len() as f32;
        discount_sum += trader.discount;
        if hunger.is_starving() {
            starving += 1;
        }
        if claim.deposit.is_some() {
            claimed += 1;
        }
        count += 1;
    }
    let denom = count.max(1) as f32;
    let avg_appetite = appetite_sum / denom;
    let avg_discount = discount_sum / denom;
    let starving_pct = starving as f32 / denom * 100.0;
    let n_deposits = world.deposits.len();

    if let Ok(mut text) = hud.single_mut() {
        let pause_tag = if paused.0 { "[PAUSED]  " } else { "" };
        let out = format!(
            "{pause_tag}econ-sim  seed {:#x}  noots {}  tick {}  trades {}  in circulation ₦{:.0}\n\
             {}/{} deposits claimed   avg appetite {:.1}   avg discount {:.2}\n\
             starving {}/{} ({:.0}%)   production {:.2}/t   consumption {:.2}/t\n\
             trade margin ₦{:.2}/t   utility {:.2}/t\n\
             deaths {:.2e}/t → target {:.2e}   hunger rate {:.2}\n\
             income ₦{:.2}/t   sales infl {:+.3}% → target {:.3}%\n\
             drag to pan · pinch to zoom · tap a noot/deposit · V/T/N overlays · S save · G new\n\n",
            world.seed, count, stats.ticks, stats.trades_total, total_bucks, claimed, n_deposits,
            avg_appetite, avg_discount, starving, count, starving_pct, stats.production_rate,
            stats.consumption_rate, stats.merchant_profit_rate, stats.utility_rate,
            hunger_ctrl.measured_per_tick, hunger_ctrl.target_per_tick, hunger_ctrl.rate,
            income_ctrl.rate, income_ctrl.measured_inflation * 100.0,
            economy::TARGET_INFLATION * 100.0
        );
        text.0 = out;
    }

    // Per-resource rows, each beside its element icon.
    for (line, mut text) in &mut lines {
        let want = resource_line(world, &stats, line.slot);
        if text.0 != want {
            text.0 = want;
        }
    }
}

/// One resource's HUD line (the leading icon identifies which element it is).
fn resource_line(world: &World, stats: &EconStats, slot: usize) -> String {
    let ce = &world.chosen[slot];
    let elem = elements::element(ce.id);
    let good = &world.goods.goods[slot];
    let category = match good.category {
        GoodCategory::Staple => "staple",
        GoodCategory::Positional => "posit ",
    };
    let (form, good_name) = match good.form {
        GoodForm::Raw => ("raw    ", elem.name),
        GoodForm::Refined => ("refined", elem.refined),
    };
    let resource = match ce.role {
        ResourceRole::Replenishable => "REPL",
        ResourceRole::Finite => "FIN ",
    };
    let avail: f64 = world
        .deposits
        .iter()
        .filter(|d| d.element_slot == slot)
        .map(|d| d.available())
        .sum();
    let item = goods::item_index(slot, good.form);
    let price = stats.ewma_price[item];
    let tail = match world.remaining_fraction(slot) {
        Some(frac) => format!("left {:>3.0}%", frac * 100.0),
        None => format!("stock {:>4.0}", avail),
    };
    format!("{good_name:<9} {category}/{form}  {resource}  ₦{price:>3.0}  {tail}")
}
