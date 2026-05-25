mod economy;
mod elements;
mod goods;
mod hex;
mod movement;
mod noot;
mod rng;
mod world;

use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::input::touch::Touch;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use economy::EconStats;
use goods::{GoodCategory, GoodForm};
use movement::tile_to_pixel;
use noot::{
    Brain, HaulContract, HaulState, Home, Hunger, Inventory, Positional, Role, TilePos, Wallet,
    N_POSITIONAL, PRINCIPAL_SHARE, STARTING_BUCKS,
};
use rng::Rng;
use world::{generate, ResourceRole, Terrain, World};

// --- World generation knobs -------------------------------------------------
const COLS: i32 = 30;
const ROWS: i32 = 22;
const HEX_SIZE: f32 = 26.0;

// --- Population -------------------------------------------------------------
const N_REFINERS: usize = 6;
const N_CONSUMERS: usize = 32;
const N_TRANSPORTERS: usize = 6;

// --- Camera limits ----------------------------------------------------------
const MIN_ZOOM: f32 = 0.3;
const MAX_ZOOM: f32 = 8.0;

// --- Selection / follow -----------------------------------------------------
/// Max screen-pixels a touch may move and still count as a tap (not a pan).
const TAP_SLOP: f32 = 12.0;
/// Min single-finger pan delta (screen px) that releases the follow lock.
const DESELECT_PAN_SLOP: f32 = 1.5;

#[derive(Resource)]
pub struct Sim(pub World);

#[derive(Resource)]
pub struct SimRng(pub Rng);

/// How tile coordinates map to world pixels (map centred on the origin).
#[derive(Resource, Clone, Copy)]
pub struct MapView {
    pub offset: Vec2,
    pub hex_size: f32,
}

/// The noot currently selected and followed, if any.
#[derive(Resource, Default)]
struct Selection(Option<Entity>);

#[derive(Component)]
struct HudText;

/// Text of the bottom panel describing the selected noot.
#[derive(Component)]
struct SelectionText;

/// The highlight ring drawn around the selected noot.
#[derive(Component)]
struct SelectionRing;

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
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                // Nested into sub-tuples purely to stay under the 20-system
                // tuple-arity limit; grouping imposes no ordering.
                (simulate, economy::income, economy::hunger_tick),
                (
                    economy::haul_assign,
                    movement::movement,
                    movement::haul_movement,
                ),
                (
                    economy::extract,
                    economy::haul_loading,
                    economy::refine,
                    economy::meet_and_trade,
                    economy::haul_settle,
                ),
                (economy::consume, economy::update_rates),
                (
                    pick_selection,
                    touch_camera,
                    keyboard_mouse_camera,
                    follow_selected,
                ),
                (
                    update_hud,
                    update_selection_ring,
                    update_selection_panel,
                    hide_loading_screen,
                ),
            ),
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
) {
    let world = generate(random_seed(), COLS, ROWS, HEX_SIZE);
    let hex_size = world.hex_size;

    // Centre the map on the origin and pick an initial zoom that fits a typical
    // phone screen in portrait, so the whole world is visible on first load.
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
    let init_zoom = (map_w / 400.0).max(map_h / 800.0).clamp(MIN_ZOOM, MAX_ZOOM);

    commands.spawn((Camera2d, Transform::from_scale(Vec3::splat(init_zoom))));

    // Shared tile mesh + per-terrain materials.
    let hex_mesh = meshes.add(RegularPolygon::new(hex_size * 0.96, 6));
    let easy_mat = materials.add(Color::srgb(0.16, 0.28, 0.20));
    let difficult_mat = materials.add(Color::srgb(0.34, 0.24, 0.17));
    for tile in &world.tiles {
        let (x, y) = hex::hex_center(tile.col, tile.row, hex_size);
        let material = match tile.terrain {
            Terrain::Easy => easy_mat.clone(),
            Terrain::Difficult => difficult_mat.clone(),
        };
        commands.spawn((
            Mesh2d(hex_mesh.clone()),
            MeshMaterial2d(material),
            Transform::from_xyz(x + offset.x, y + offset.y, 0.0),
        ));
    }

    // Deposit markers, coloured by their element.
    let deposit_mesh = meshes.add(RegularPolygon::new(hex_size * 0.5, 6));
    for deposit in &world.deposits {
        let tile = &world.tiles[deposit.tile];
        let (x, y) = hex::hex_center(tile.col, tile.row, hex_size);
        let (r, g, b) = elements::element(world.chosen[deposit.element_slot].id).color;
        commands.spawn((
            Mesh2d(deposit_mesh.clone()),
            MeshMaterial2d(materials.add(Color::srgb(r, g, b))),
            Transform::from_xyz(x + offset.x, y + offset.y, 1.0),
        ));
    }

    commands.insert_resource(MapView { offset, hex_size });

    // Highlight ring for the selected noot (hidden until something is picked).
    let ring_mesh = meshes.add(Annulus::new(hex_size * 0.34, hex_size * 0.46));
    commands.spawn((
        Mesh2d(ring_mesh),
        MeshMaterial2d(materials.add(Color::srgb(1.0, 0.95, 0.3))),
        Transform::from_xyz(0.0, 0.0, 2.5),
        Visibility::Hidden,
        SelectionRing,
    ));

    // Spawn the noots.
    let mut sim_rng = Rng::new(world.seed ^ 0xA5A5_5A5A);
    let noot_mesh = meshes.add(Circle::new(hex_size * 0.28));
    let owner_mat = materials.add(Color::srgb(0.95, 0.78, 0.25));
    let refiner_mat = materials.add(Color::srgb(0.30, 0.60, 0.95));
    let consumer_mat = materials.add(Color::srgb(0.40, 0.85, 0.45));
    let transporter_mat = materials.add(Color::srgb(0.85, 0.45, 0.85));

    // One owner seeded onto each deposit (so extraction can start).
    for di in 0..world.deposits.len() {
        let tile = world.deposits[di].tile;
        let (col, row) = (world.tiles[tile].col, world.tiles[tile].row);
        spawn_noot(
            &mut commands,
            noot_mesh.clone(),
            owner_mat.clone(),
            Role::Owner { deposit: di },
            col,
            row,
            sim_rng.below(6),
            tile_to_pixel(col, row, hex_size, offset),
        );
    }
    // Refiners and consumers at random tiles.
    for _ in 0..N_REFINERS {
        let (col, row) = random_tile(&mut sim_rng, &world);
        spawn_noot(
            &mut commands,
            noot_mesh.clone(),
            refiner_mat.clone(),
            Role::Refiner,
            col,
            row,
            sim_rng.below(6),
            tile_to_pixel(col, row, hex_size, offset),
        );
    }
    for _ in 0..N_CONSUMERS {
        let (col, row) = random_tile(&mut sim_rng, &world);
        spawn_noot(
            &mut commands,
            noot_mesh.clone(),
            consumer_mat.clone(),
            Role::Consumer,
            col,
            row,
            sim_rng.below(6),
            tile_to_pixel(col, row, hex_size, offset),
        );
    }
    // Transporters: hired haulers, spawned idle at random tiles.
    for _ in 0..N_TRANSPORTERS {
        let (col, row) = random_tile(&mut sim_rng, &world);
        commands.spawn((
            Mesh2d(noot_mesh.clone()),
            MeshMaterial2d(transporter_mat.clone()),
            Transform::from_xyz(
                tile_to_pixel(col, row, hex_size, offset).x,
                tile_to_pixel(col, row, hex_size, offset).y,
                2.0,
            ),
            Role::Transporter,
            TilePos { col, row },
            Home { col, row },
            Inventory::new(),
            Wallet {
                bucks: STARTING_BUCKS,
            },
            Hunger::starving(),
            Positional {
                stock: [0.0; N_POSITIONAL],
            },
            Brain::new(sim_rng.below(6)),
            HaulContract::idle(),
        ));
    }

    spawn_ui(&mut commands);

    commands.insert_resource(SimRng(sim_rng));
    commands.insert_resource(Sim(world));
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
    mesh: Handle<Mesh>,
    material: Handle<ColorMaterial>,
    role: Role,
    col: i32,
    row: i32,
    heading: usize,
    pixel: Vec2,
) {
    commands.spawn((
        Mesh2d(mesh),
        MeshMaterial2d(material),
        Transform::from_xyz(pixel.x, pixel.y, 2.0),
        role,
        TilePos { col, row },
        Home { col, row },
        Inventory::new(),
        Wallet {
            bucks: STARTING_BUCKS,
        },
        Hunger::starving(),
        Positional {
            stock: [0.0; N_POSITIONAL],
        },
        Brain::new(heading),
    ));
}

fn spawn_ui(commands: &mut Commands) {
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
            // Status panel (top).
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
                    Text::new("loading..."),
                    TextFont {
                        font_size: 14.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    HudText,
                ));
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
                        font_size: 13.0,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    SelectionText,
                ));
            });
        });
}

fn simulate(time: Res<Time>, mut sim: ResMut<Sim>) {
    sim.0.tick(time.delta_secs());
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

/// A tap (touch) or left-click that didn't pan selects the nearest noot under
/// the pointer; an empty hit clears the selection.
fn pick_selection(
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    noots: Query<(Entity, &Transform, &Role)>,
    view: Res<MapView>,
    mut selection: ResMut<Selection>,
) {
    let Ok((camera, cam_tf)) = cameras.single() else {
        return;
    };

    // Collect this frame's pick points in screen space.
    let mut points: Vec<Vec2> = Vec::new();
    // Desktop: no mouse-drag panning exists, so any left click is a pick.
    if mouse.just_pressed(MouseButton::Left) {
        if let Ok(window) = windows.single() {
            if let Some(cursor) = window.cursor_position() {
                points.push(cursor);
            }
        }
    }
    // Mobile: a tap is a touch that lifted with little movement (a drag pans).
    for touch in touches.iter_just_released() {
        if (touch.position() - touch.start_position()).length() < TAP_SLOP {
            points.push(touch.position());
        }
    }
    if points.is_empty() {
        return;
    }

    let pick_r2 = (view.hex_size * 0.6).powi(2);
    for screen in points {
        let Ok(world_pos) = camera.viewport_to_world_2d(cam_tf, screen) else {
            continue;
        };
        let mut best: Option<(Entity, f32)> = None;
        for (e, tf, _role) in &noots {
            let d2 = tf.translation.truncate().distance_squared(world_pos);
            if d2 <= pick_r2 && best.map_or(true, |(_, bd)| d2 < bd) {
                best = Some((e, d2));
            }
        }
        selection.0 = best.map(|(e, _)| e);
    }
}

/// Keep the camera centred on the selected noot (a manual pan clears it).
fn follow_selected(
    selection: Res<Selection>,
    noots: Query<&Transform, (With<Role>, Without<Camera2d>)>,
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
    noots: Query<&Transform, (With<Role>, Without<SelectionRing>)>,
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
fn update_selection_panel(
    selection: Res<Selection>,
    sim: Res<Sim>,
    noots: Query<(
        &Role,
        &Wallet,
        &Hunger,
        &Positional,
        &Inventory,
        Option<&HaulContract>,
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
    let Ok((role, wallet, hunger, positional, inv, contract)) = noots.get(entity) else {
        text.0 = stale.into();
        return;
    };

    let world = &sim.0;
    let role_label = match role {
        Role::Owner { deposit } => {
            let slot = world.deposits[*deposit].element_slot;
            format!("owner of {}", elements::element(world.chosen[slot].id).name)
        }
        Role::Refiner => "refiner".to_string(),
        Role::Consumer => "consumer".to_string(),
        Role::Transporter => {
            let state = contract.map_or("idle", |c| match c.state {
                HaulState::Idle => "idle",
                HaulState::ToPickup => "to pickup",
                HaulState::Loading => "loading",
                HaulState::Selling => "selling",
                HaulState::Returning => "returning",
            });
            format!("transporter — {}", state)
        }
    };

    let utility = hunger.utility() + positional.utility();
    let mut out = format!(
        "[selected] {}   ₦{:.0}   hunger {:.1}   utility {:.2}\n",
        role_label,
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
    if let Some(c) = contract {
        if c.state != HaulState::Idle {
            out.push_str(&format!(
                "cargo: {}   take ₦{:.0}   owner's share {:.0}%\n",
                held,
                c.proceeds,
                PRINCIPAL_SHARE * 100.0
            ));
        } else {
            out.push_str(&format!("holding: {}\n", held));
        }
    } else {
        out.push_str(&format!("holding: {}\n", held));
    }
    text.0 = out;
}

fn update_hud(
    sim: Res<Sim>,
    stats: Res<EconStats>,
    noots: Query<(&Role, &Wallet, &Hunger)>,
    haulers: Query<&HaulContract>,
    mut hud: Query<&mut Text, With<HudText>>,
) {
    let world = &sim.0;

    // Aggregate noot stats.
    let (mut owners, mut refiners, mut consumers, mut transporters) = (0u32, 0u32, 0u32, 0u32);
    let mut total_bucks = 0.0f32;
    let mut appetite_sum = 0.0f32;
    let mut starving = 0u32;
    let mut count = 0u32;
    let mut eaters = 0u32; // noots that actually consume (everyone but transporters)
    for (role, wallet, hunger) in &noots {
        match role {
            Role::Owner { .. } => owners += 1,
            Role::Refiner => refiners += 1,
            Role::Consumer => consumers += 1,
            Role::Transporter => transporters += 1,
        }
        total_bucks += wallet.bucks;
        count += 1;
        // Transporters don't eat, so they're not part of hunger/starvation stats.
        if !matches!(role, Role::Transporter) {
            appetite_sum += hunger.staple.iter().sum::<f32>() / hunger.staple.len() as f32;
            if hunger.is_starving() {
                starving += 1;
            }
            eaters += 1;
        }
    }
    let active_hauls = haulers
        .iter()
        .filter(|c| c.state != HaulState::Idle)
        .count();
    let avg_appetite = if eaters > 0 {
        appetite_sum / eaters as f32
    } else {
        0.0
    };
    let starving_pct = if eaters > 0 {
        starving as f32 / eaters as f32 * 100.0
    } else {
        0.0
    };

    if let Ok(mut text) = hud.single_mut() {
        let mut out = format!(
            "econ-sim  seed {:#x}  noots {}  trades {}  in circulation ₦{:.0}\n\
             {} owners · {} refiners · {} consumers · {} transporters   avg appetite {:.1}\n\
             starving {}/{} ({:.0}%)   production {:.1}/s   consumption {:.1}/s\n\
             haulers {}/{} active   hauled {:.1}/s   utility {:.1}/s\n\
             drag to pan · pinch to zoom · tap a noot to follow it\n\n",
            world.seed, count, stats.trades_total, total_bucks, owners, refiners, consumers,
            transporters, avg_appetite, starving, eaters, starving_pct, stats.production_rate,
            stats.consumption_rate, active_hauls, transporters, stats.hauled_rate,
            stats.utility_rate
        );
        for slot in 0..4 {
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
            let price = stats.last_price[item];
            let tail = match world.remaining_fraction(slot) {
                Some(frac) => format!("left {:>3.0}%", frac * 100.0),
                None => format!("stock {:>4.0}", avail),
            };
            out.push_str(&format!(
                "{}. {:<9} {}/{}  {}  ₦{:>3.0}  {}\n",
                slot + 1,
                good_name,
                category,
                form,
                resource,
                price,
                tail
            ));
        }
        text.0 = out;
    }
}
