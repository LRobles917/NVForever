//! Soft edges and contact shadows for the lamp shadows (`light_shadows.rs`; the settings
//! row ALC shadows; docs/design/graphics-enhanced.md §3.17 "Soft edges and contact
//! shadows"). Both only on Quality [engineering choice: each costs per pixel and per
//! light; Performance keeps the hard edges of the camera's shadow filter].
//!
//! - **Soft edges**: every casting point light (`CastingLight`) takes percentage-closer soft
//!   shadows (`soft_shadows_enabled`, the `experimental_pbr_pcss` feature for its samplers)
//!   with a light size of [`SOFT_RADIUS`] (an explosion's [`BLAST_RADIUS`]): sharp where a
//!   shape meets the surface its shadow falls on, softer further away, as a lamp's are. The
//!   size goes to the renderer apart from the light's `radius`
//!   (`HoldLightShadowMaps::soft_size`), so the lighting does not change. The filter
//!   is the patched renderer's own (bevy_pbr shadow_sampling.wesl
//!   `nvf_sample_shadow_cubemap_soft`, engine/vendor/README.md "ALCS soft shadows"): the
//!   blocker search and the penumbra in world units, each tap compared against the
//!   fragment's own surface. Bevy's takes the light's size as a plain number, a fraction of
//!   a metre in mind: with the game's units its search swept most of the cube and every lit
//!   surface shadowed itself in stripes (playtest 2026-11-24). Spot lamps keep the camera's
//!   shadow filter (Bevy's soft shadows for them have the same fault).
//! - **Contact shadows**: the world camera takes Bevy's `ContactShadows` (a ray march
//!   along the depth buffer towards each light, [`CONTACT`]), and every drawn placed light,
//!   grass fire and attached light within the sun's shadow distance has
//!   `contact_shadows_enabled`: small shadows where things touch (a bottle on a bar, a foot
//!   on a floor) that the lamps' 512-texel maps miss, and the only shadows of the lights
//!   past the cap. The camera then needs a depth prepass (`DepthPrepass`, which
//!   `ContactShadows` requires), as FSR and DLSS already do. Only while a temporal
//!   method is on (TAA, FSR or DLSS: the camera jitters): without one the march's
//!   per-pixel noise speckles walls and ceilings [observed 2026-11-18, General Store
//!   pictures]. With FSR or DLSS the picture is drawn below its output size into the
//!   top-left of full-size textures (`MainPassResolutionOverride`); Bevy's march read the
//!   whole depth texture as the view and threw hard-edged false shadows that changed as
//!   the camera moved [observed 2026-11-18: playtest videos, a walk along the
//!   Prospector Saloon's porch]. The vendored march now reads the main pass's viewport
//!   only (bevy_pbr raymarch.wesl `depth_uv_scale`, engine/vendor/README.md).
//! - The sun keeps neither: its soft edges are the cascades' filter, and Bevy's contact
//!   shadows on it speckled near surfaces at a grazing angle (§4, tried and dropped).

use bevy::camera::visibility::VisibilitySystems;
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::light::SimulationLightSystems;
use bevy::pbr::ContactShadows;
use bevy::ecs::entity::EntityHashMap;
use bevy::pbr::HoldLightShadowMaps;
use bevy::prelude::*;
use bevy::render::camera::TemporalJitter;
use bevy::transform::TransformSystems;

use crate::light_shadows::{CastingLight, LightShadows, ShadowSource, SourceKind};
use crate::lights::SessionLight;

/// The light size of a lamp's soft shadow (game units, about 34 cm: a shade, a lantern)
/// [engineering choice, compared in pictures 2026-11-18 in the Goodsprings General Store:
/// at 8 the edges looked as hard as without; at 24-32 a hanging wire's shadow is soft away
/// from the ceiling and sharp where it touches it].
pub const SOFT_RADIUS: f32 = 24.0;

/// An explosion's light size (game units, about 70 cm: a fireball) [engineering choice].
pub const BLAST_RADIUS: f32 = 48.0;

/// The contact shadows' ray march: steps, the assumed thickness of what the depth buffer
/// shows, and the ray's length (game units: 4 and 24, about 6 and 34 cm) [engineering
/// choice: short and thin, unlike the sun's dropped 40 and 12, so a surface seen at a
/// grazing angle does not shadow itself].
pub const CONTACT: ContactShadows = ContactShadows { linear_steps: 8, thickness: 4.0, length: 24.0 };

/// `NVF_LIGHT_SHADOW_SOFT=<size>` / `NVF_LIGHT_SHADOW_CONTACT=0` (test aids): another light size for
/// the soft edges (0: none), no contact shadows.
fn soft_aid() -> Option<f32> {
    static V: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("NVF_LIGHT_SHADOW_SOFT").ok().and_then(|v| v.trim().parse::<f32>().ok()))
}

fn contact_aid_off() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("NVF_LIGHT_SHADOW_CONTACT").is_ok_and(|v| v.trim() == "0"))
}

/// [`CONTACT`], or `NVF_LIGHT_SHADOW_CONTACT_MARCH=<steps>,<thickness>,<length>` (a test aid).
fn contact_settings() -> ContactShadows {
    std::env::var("NVF_LIGHT_SHADOW_CONTACT_MARCH")
        .ok()
        .and_then(|v| {
            let p: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            (p.len() == 3).then(|| ContactShadows { linear_steps: p[0] as u32, thickness: p[1], length: p[2] })
        })
        .unwrap_or(CONTACT)
}

/// Whether the choice (`LightShadows::choice`) is Quality.
pub fn detailed(choice: usize) -> bool {
    choice == 2
}

/// The soft-shadow size of a casting light by what it is.
pub fn soft_radius(kind: Option<SourceKind>) -> f32 {
    match kind {
        Some(SourceKind::Explosion) => BLAST_RADIUS,
        _ => SOFT_RADIUS,
    }
}

pub struct LightShadowDetailPlugin;

impl Plugin for LightShadowDetailPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            (soften, contact)
                .after(TransformSystems::Propagate)
                .before(VisibilitySystems::VisibilityPropagate)
                .before(SimulationLightSystems::AssignLightsToClusters)
                // With the row Off nothing of the ALC shadows runs; for a few frames after
                // it is switched off these give the soft edges and contact shadows back.
                .run_if(crate::light_shadows::live),
        );
    }
}

type Lit<'a> = (Entity, Has<PointLight>, Has<CastingLight>, Option<&'a ShadowSource>);

/// The lights these settings touch: the placed ones and the other sources.
type Lamps = Or<(With<SessionLight>, With<ShadowSource>)>;

type Cameras<'a> = (Entity, &'a GlobalTransform, Has<ContactShadows>, Has<TemporalJitter>);

/// The soft sizes of the casting point lights on Quality, and of those whose held maps still
/// fade out (a slot kept, `HoldLightShadowMaps`); none otherwise. They go to the renderer
/// in `HoldLightShadowMaps::soft_size`, apart from the lights' `radius`: until 2026-11-24
/// the radius carried the size, and the lighting read it too (a lamp's highlight grew and
/// dimmed, and popped when the size was switched). Spot lamps get none: they keep the
/// camera's shadow filter.
fn soften(settings: Res<LightShadows>, mut hold: ResMut<HoldLightShadowMaps>, lights: Query<Lit, Lamps>) {
    let _timed = crate::light_shadows::Timed::start(2);
    let mut sizes: EntityHashMap<f32> = EntityHashMap::default();
    if detailed(settings.choice) {
        for (e, point, casting, source) in &lights {
            let size = soft_aid().unwrap_or_else(|| soft_radius(source.map(|s| s.0)));
            if point && size > 0.0 && (casting || hold.slots.contains_key(&e)) {
                sizes.insert(e, size);
            }
        }
    }
    if hold.soft_size != sizes {
        hold.soft_size = sizes;
    }
}
type Contacted<'a> = (&'a GlobalTransform, Option<&'a mut PointLight>, Option<&'a mut SpotLight>, Option<&'a ShadowSource>);

/// The camera's contact shadows on Quality, and the lights that take them.
#[allow(clippy::too_many_arguments)]
fn contact(
    mut commands: Commands,
    settings: Res<LightShadows>,
    sky: Option<Res<crate::sky::SkyState>>,
    cameras: Query<Cameras, crate::fly::WorldCamera>,
    mut lights: Query<Contacted, Lamps>,
    mut logged: Local<Option<(bool, usize)>>,
) {
    let _timed = crate::light_shadows::Timed::start(3);
    let mut on = false;
    let mut at = None;
    for (e, t, has, temporal) in &cameras {
        at = Some(t.translation());
        // Only with a temporal method (TAA, FSR, DLSS: the camera jitters), which smooths
        // the march's per-pixel noise; without one it speckles the walls (§4). Under FSR and
        // DLSS the vendored march reads the depth of the main pass's viewport only
        // (raymarch.wesl `depth_uv_scale`): before that it threw hard-edged false shadows
        // (playtest videos 2026-11-18, the Prospector Saloon's porch).
        on = detailed(settings.choice) && !contact_aid_off() && temporal;
        if on && !has {
            commands.entity(e).insert((contact_settings(), DepthPrepass));
        } else if !on && has {
            // The depth prepass stays: FSR and DLSS may hold it too, and it is harmless.
            commands.entity(e).remove::<ContactShadows>();
        }
    }
    let reach = sky.as_ref().map_or(0.0, |s| s.shadow_distance * s.shadow_reach.max(1.0));
    let mut count = 0;
    for (t, point, spot, source) in &mut lights {
        // Not the transients: a tenth of a second, and their maps are drawn anyway.
        let kind_ok = source.is_none_or(|s| !s.0.transient());
        let range = point.as_ref().map(|p| p.range).or(spot.as_ref().map(|s| s.range)).unwrap_or(0.0);
        let near = at.is_some_and(|a| (t.translation().distance(a) - range).max(0.0) <= reach);
        let want = on && kind_ok && near;
        count += usize::from(want);
        if let Some(mut p) = point {
            if p.contact_shadows_enabled != want {
                p.contact_shadows_enabled = want;
            }
        }
        if let Some(mut s) = spot {
            if s.contact_shadows_enabled != want {
                s.contact_shadows_enabled = want;
            }
        }
    }
    let soft = detailed(settings.choice);
    if *logged != Some((on, count)) && (soft || logged.is_some()) {
        *logged = Some((on, count));
        match (soft, on) {
            (true, true) => info!(
                "light shadows: soft edges (light size {SOFT_RADIUS}, explosions {BLAST_RADIUS}) and contact shadows on {count} lights ({} steps, thickness {}, length {})",
                CONTACT.linear_steps, CONTACT.thickness, CONTACT.length
            ),
            (true, false) => info!("light shadows: soft edges (light size {SOFT_RADIUS}, explosions {BLAST_RADIUS}); no contact shadows (only with TAA, FSR or DLSS)"),
            _ => info!("light shadows: no soft edges or contact shadows"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_quality_softens_and_explosions_are_larger() {
        assert!(detailed(2));
        assert!(!detailed(1) && !detailed(0));
        assert_eq!(soft_radius(None), SOFT_RADIUS);
        assert_eq!(soft_radius(Some(SourceKind::MuzzleFlash)), SOFT_RADIUS);
        assert_eq!(soft_radius(Some(SourceKind::Explosion)), BLAST_RADIUS);
        let c = contact_settings();
        assert!(c.length < 40.0 && c.thickness < 12.0, "shorter and thinner than the sun's dropped try");
    }
}
