use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::error::{ThermalError, ThermalResult};

// ---------------------------------------------------------------------------
// Domain types (used throughout the thermal model)
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct RadiatorDef {
    pub t50: f64,
    pub active: bool,
    pub pipe: &'static str,
}

#[derive(Clone)]
pub(crate) struct ExternalElement {
    pub description: &'static str,
    pub area: f64,
    pub u_value: f64,
    pub to_ground: bool,
}

#[derive(Clone)]
pub(crate) struct SolarGlazingDef {
    pub area: f64,
    pub orientation: &'static str,
    pub tilt: &'static str,
    pub g_value: f64,
    pub shading: f64,
}

#[allow(dead_code)]
#[derive(Clone)]
pub(crate) struct RoomDef {
    pub name: &'static str,
    pub floor: &'static str,
    pub floor_area: f64,
    pub ceiling_height: f64,
    pub construction: &'static str,
    pub radiators: Vec<RadiatorDef>,
    pub external_fabric: Vec<ExternalElement>,
    pub solar: Vec<SolarGlazingDef>,
    pub sensor_topic: &'static str,
    pub ventilation_ach: f64,
    pub heat_recovery: f64,
    pub overnight_occupants: i32,
}

#[derive(Clone)]
pub(crate) struct InternalConnection {
    pub room_a: &'static str,
    pub room_b: &'static str,
    pub ua: f64,
    pub description: &'static str,
}

#[derive(Clone)]
pub(crate) struct Doorway {
    pub room_a: &'static str,
    pub room_b: &'static str,
    pub width: f64,
    pub height: f64,
    pub state: &'static str,
}

// ---------------------------------------------------------------------------
// JSON geometry file types (serde)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct GeometryFile {
    pub rooms: Vec<GeometryRoom>,
    pub connections: Vec<GeometryConnection>,
    pub doorways: Vec<GeometryDoorway>,
}

#[derive(Debug, Deserialize)]
struct GeometrySolarGlazing {
    area: f64,
    orientation: String,
    #[serde(default = "default_vertical")]
    tilt: String,
    #[serde(default = "default_g_value")]
    g_value: f64,
    #[serde(default = "default_shading")]
    shading: f64,
}

fn default_vertical() -> String {
    "vertical".to_string()
}
fn default_g_value() -> f64 {
    0.7
}
fn default_shading() -> f64 {
    1.0
}

#[derive(Debug, Deserialize)]
pub(crate) struct GeometryRoom {
    pub name: String,
    pub floor: String,
    pub floor_area: f64,
    pub ceiling_height: f64,
    pub construction: String,
    pub sensor: String,
    pub ventilation_ach: f64,
    pub heat_recovery: f64,
    pub overnight_occupants: i32,
    radiators: Vec<GeometryRadiator>,
    external_fabric: Vec<GeometryExternalElement>,
    #[serde(default)]
    solar: Vec<GeometrySolarGlazing>,
}

#[derive(Debug, Deserialize)]
struct GeometryRadiator {
    t50: f64,
    #[serde(default = "default_true")]
    active: bool,
    #[serde(default = "default_pipe")]
    pipe: String,
}

fn default_pipe() -> String {
    "none".to_string()
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct GeometryExternalElement {
    description: String,
    area: f64,
    u_value: f64,
    #[serde(default)]
    to_ground: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GeometryConnection {
    pub room_a: String,
    pub room_b: String,
    pub ua: f64,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GeometryDoorway {
    pub room_a: String,
    pub room_b: String,
    pub width: f64,
    pub height: f64,
    pub state: String,
}

// ---------------------------------------------------------------------------
// Loading and building
// ---------------------------------------------------------------------------

fn thermal_geometry_path() -> PathBuf {
    Path::new("data/canonical/thermal_geometry.json").to_path_buf()
}

pub(crate) fn load_thermal_geometry() -> ThermalResult<GeometryFile> {
    let path = thermal_geometry_path();
    let txt = fs::read_to_string(&path).map_err(|source| ThermalError::ConfigRead {
        path: path.display().to_string(),
        source,
    })?;
    serde_json::from_str(&txt).map_err(|source| ThermalError::GeometryParse {
        path: path.display().to_string(),
        source,
    })
}

pub(crate) fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

pub(crate) fn build_rooms() -> ThermalResult<BTreeMap<String, RoomDef>> {
    let geo = load_thermal_geometry()?;
    let mut rooms = BTreeMap::new();

    for r in geo.rooms {
        let name = leak(r.name);
        let room = RoomDef {
            name,
            floor: leak(r.floor),
            floor_area: r.floor_area,
            ceiling_height: r.ceiling_height,
            construction: leak(r.construction),
            radiators: r
                .radiators
                .into_iter()
                .map(|rad| RadiatorDef {
                    t50: rad.t50,
                    active: rad.active,
                    pipe: leak(rad.pipe),
                })
                .collect(),
            external_fabric: r
                .external_fabric
                .into_iter()
                .map(|e| ExternalElement {
                    description: leak(e.description),
                    area: e.area,
                    u_value: e.u_value,
                    to_ground: e.to_ground,
                })
                .collect(),
            solar: r
                .solar
                .into_iter()
                .map(|s| SolarGlazingDef {
                    area: s.area,
                    orientation: leak(s.orientation),
                    tilt: leak(s.tilt),
                    g_value: s.g_value,
                    shading: s.shading,
                })
                .collect(),
            sensor_topic: leak(r.sensor),
            ventilation_ach: r.ventilation_ach,
            heat_recovery: r.heat_recovery,
            overnight_occupants: r.overnight_occupants,
        };
        rooms.insert(name.to_string(), room);
    }

    Ok(rooms)
}

pub(crate) fn build_connections() -> ThermalResult<Vec<InternalConnection>> {
    let geo = load_thermal_geometry()?;
    Ok(geo
        .connections
        .into_iter()
        .map(|c| InternalConnection {
            room_a: leak(c.room_a),
            room_b: leak(c.room_b),
            ua: c.ua,
            description: leak(c.description),
        })
        .collect())
}

pub(crate) fn build_doorways() -> ThermalResult<Vec<Doorway>> {
    let geo = load_thermal_geometry()?;
    Ok(geo
        .doorways
        .into_iter()
        .map(|d| Doorway {
            room_a: leak(d.room_a),
            room_b: leak(d.room_b),
            width: d.width,
            height: d.height,
            state: leak(d.state),
        })
        .collect())
}
