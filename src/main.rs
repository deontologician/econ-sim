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

use economy::EconStats;
use goods::{GoodCategory, GoodForm};
use movement::tile_to_pixel;
use noot::{
    Brain, Home, Hunger, Inventory, Positional, Role, TilePos, Wallet, N_POSITIONAL, STARTING_BUCKS,
};
use rng::Rng;
use world::{generate, ResourceRole, Terrain, World};

// --- World generation knobs -------------------------------------------------
// The seed is fixed so a build is reproducible; change it for a different world.
const SEED: u64 = 0xC0FFEE_1234;
const COLS: i32 = 30;
const ROWS: i32 = 22;
const HEX_SIZE: f32 = 26.0;

// --- Population -------------------------------------------------------------
const N_REFINERS: usize = 6;
const N_CONSUMERS: usize = 32;

// --- Camera limits ----------------------------------------------------------
const MIN_ZOOM: f32 = 0.3;
const MAX_ZOOM: f32 = 8.0;

// --- Button colours ---------------------------------------------------------
const BTN_IDLE: Color = Color::srgba(0.12, 0.12, 0.15, 0.88);
const BTN_HOVER: Color = Color::srgba(0.20, 0.20, 0.26, 0.92);
const BTN_PRESSED: Color = Color::srgba(0.28, 0.45, 0.30, 0.95);

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

#[derive(Component)]
struct HudText;

#[derive(Component)]
struct ElementButton(usize);

#[derive(Component)]
struct ButtonLabel(usize);

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
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                simulate,
                economy::income,
                economy::hunger_tick,
                movement::movement,
                economy::extract,
                economy::refine,
                economy::meet_and_trade,
                economy::consume,
                touch_camera,
                keyboard_mouse_camera,
                invest_buttons,
                update_hud,
            ),
        )
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let world = generate(SEED, COLS, ROWS, HEX_SIZE);
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

    // Spawn the noots.
    let mut sim_rng = Rng::new(world.seed ^ 0xA5A5_5A5A);
    let noot_mesh = meshes.add(Circle::new(hex_size * 0.28));
    let owner_mat = materials.add(Color::srgb(0.95, 0.78, 0.25));
    let refiner_mat = materials.add(Color::srgb(0.30, 0.60, 0.95));
    let consumer_mat = materials.add(Color::srgb(0.40, 0.85, 0.45));

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

            // Invest buttons (bottom, thumb-reachable).
            root.spawn(Node {
                width: Val::Percent(100.0),
                column_gap: Val::Px(6.0),
                ..default()
            })
            .with_children(|row| {
                for slot in 0..4usize {
                    row.spawn((
                        Button,
                        ElementButton(slot),
                        Node {
                            flex_grow: 1.0,
                            flex_basis: Val::Px(0.0),
                            height: Val::Px(58.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            padding: UiRect::all(Val::Px(4.0)),
                            ..default()
                        },
                        BackgroundColor(BTN_IDLE),
                    ))
                    .with_children(|button| {
                        button.spawn((
                            Text::new(""),
                            TextFont {
                                font_size: 13.0,
                                ..default()
                            },
                            TextColor(Color::WHITE),
                            ButtonLabel(slot),
                        ));
                    });
                }
            });
        });
}

fn simulate(time: Res<Time>, mut sim: ResMut<Sim>) {
    sim.0.tick(time.delta_secs());
}

/// Touch: one finger drags the map, two fingers pinch to zoom (and pan).
fn touch_camera(touches: Res<Touches>, mut camera: Query<&mut Transform, With<Camera2d>>) {
    let Ok(mut transform) = camera.single_mut() else {
        return;
    };
    let scale = transform.scale.x;
    let active: Vec<&Touch> = touches.iter().collect();

    match active.as_slice() {
        [finger] => {
            pan(&mut transform, finger.delta(), scale);
        }
        [a, b, ..] => {
            let mid = (a.delta() + b.delta()) * 0.5;
            pan(&mut transform, mid, scale);

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
    }

    if scroll.delta.y != 0.0 {
        let factor = if scroll.delta.y > 0.0 { 0.9 } else { 1.1 };
        transform.scale = Vec3::splat((scale * factor).clamp(MIN_ZOOM, MAX_ZOOM));
    }
}

/// Tapping an element button (or pressing 1-4) invests tech into that element,
/// raising the efficiency of its deposits.
fn invest_buttons(
    mut buttons: Query<(&Interaction, &ElementButton, &mut BackgroundColor), Changed<Interaction>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut sim: ResMut<Sim>,
) {
    for (interaction, button, mut background) in &mut buttons {
        match *interaction {
            Interaction::Pressed => {
                invest(&mut sim.0, button.0);
                *background = BackgroundColor(BTN_PRESSED);
            }
            Interaction::Hovered => *background = BackgroundColor(BTN_HOVER),
            Interaction::None => *background = BackgroundColor(BTN_IDLE),
        }
    }

    let digits = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
    ];
    for (slot, key) in digits.iter().enumerate() {
        if keys.just_pressed(*key) {
            invest(&mut sim.0, slot);
        }
    }
}

fn invest(world: &mut World, slot: usize) {
    if let Some(element) = world.chosen.get_mut(slot) {
        element.efficiency = (element.efficiency + 0.25).min(8.0);
    }
}

fn update_hud(
    sim: Res<Sim>,
    stats: Res<EconStats>,
    noots: Query<(&Role, &Wallet, &Hunger)>,
    mut hud: Query<&mut Text, (With<HudText>, Without<ButtonLabel>)>,
    mut labels: Query<(&mut Text, &ButtonLabel), Without<HudText>>,
) {
    let world = &sim.0;

    // Aggregate noot stats.
    let (mut owners, mut refiners, mut consumers) = (0u32, 0u32, 0u32);
    let mut total_bucks = 0.0f32;
    let mut appetite_sum = 0.0f32;
    let mut count = 0u32;
    for (role, wallet, hunger) in &noots {
        match role {
            Role::Owner { .. } => owners += 1,
            Role::Refiner => refiners += 1,
            Role::Consumer => consumers += 1,
        }
        total_bucks += wallet.bucks;
        appetite_sum += hunger.staple.iter().sum::<f32>() / hunger.staple.len() as f32;
        count += 1;
    }
    let avg_appetite = if count > 0 {
        appetite_sum / count as f32
    } else {
        0.0
    };

    if let Ok(mut text) = hud.single_mut() {
        let mut out = format!(
            "econ-sim  seed {:#x}  noots {}  trades {}  in circulation ₦{:.0}\n\
             {} owners · {} refiners · {} consumers   avg appetite {:.1}\n\
             drag to pan · pinch to zoom · tap an element to invest in extraction\n\n",
            world.seed, count, stats.trades_total, total_bucks, owners, refiners, consumers,
            avg_appetite
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
                "{}. {:<9} {}/{}  {}  ₦{:>3.0}  x{:.2}  {}\n",
                slot + 1,
                good_name,
                category,
                form,
                resource,
                price,
                ce.efficiency,
                tail
            ));
        }
        text.0 = out;
    }

    for (mut text, label) in &mut labels {
        if let Some(ce) = world.chosen.get(label.0) {
            text.0 = format!(
                "{}\n+eff x{:.2}",
                elements::element(ce.id).name,
                ce.efficiency
            );
        }
    }
}
