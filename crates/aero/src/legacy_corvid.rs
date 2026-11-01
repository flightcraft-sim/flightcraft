//! Original pre-migration values, retained only as a JSON rounding regression fixture.
const DEG: f64 = core::f64::consts::PI / 180.0;
pub const MASS_KG: f64 = 1000.0;
pub const INERTIA_KG_M2: [f64; 3] = [1200.0, 1700.0, 2400.0];
pub const WING_AREA_M2: f64 = 15.0;
pub const SPAN_M: f64 = 10.0;
pub const CHORD_M: f64 = 1.5;
pub const MAX_POWER_W: f64 = 120_000.0;
pub const CL_MAX: f64 = 1.50;
pub const PROP_DIAMETER_M: f64 = 1.9;

/// Original finite-wing estimate: cambered four-digit-section shape informed by
/// NACA Report 824 (1945), with slope reduced by lifting-line theory for AR=6.67.
/// Stall and post-stall ordinates are conservative design assumptions, not digitised data.
pub const CL_CLEAN: &[(f64, f64)] = &[
    (-25.0 * DEG, -0.65),
    (-18.0 * DEG, -0.95),
    (-14.0 * DEG, -1.0),
    (-8.0 * DEG, -0.448),
    (0.0, 0.25),
    (8.0 * DEG, 0.948),
    (14.0 * DEG, 1.47),
    (16.0 * DEG, 1.50),
    (20.0 * DEG, 1.15),
    (25.0 * DEG, 0.85),
];
/// Original plain-flap estimate: +0.5 CL in attached flow, earlier separation.
pub const CL_FLAPS: &[(f64, f64)] = &[
    (-25.0 * DEG, -0.45),
    (-18.0 * DEG, -0.7),
    (-14.0 * DEG, -0.5),
    (-8.0 * DEG, 0.052),
    (0.0, 0.75),
    (8.0 * DEG, 1.448),
    (12.0 * DEG, 1.85),
    (16.0 * DEG, 1.65),
    (20.0 * DEG, 1.3),
    (25.0 * DEG, 1.0),
];
/// Original parasitic drag rise with incidence, plus CDi=0.055 CL² below.
/// k=1/(pi AR e), e≈0.87 for the complete wing.
pub const CD_ALPHA: &[(f64, f64)] = &[
    (-25.0 * DEG, 0.18),
    (-16.0 * DEG, 0.035),
    (0.0, 0.028),
    (16.0 * DEG, 0.035),
    (25.0 * DEG, 0.18),
];
/// Original static margin: Cm_alpha=-1.2/rad, Cm0=0.04, from a tail-volume build-up.
pub const CM_ALPHA: &[(f64, f64)] = &[
    (-25.0 * DEG, 0.563598776),
    (0.0, 0.04),
    (25.0 * DEG, -0.483598776),
];
/// Original vertical-tail / fuselage sideforce estimate, CY_beta=-0.65/rad.
pub const CY_BETA: &[(f64, f64)] = &[(-30.0 * DEG, 0.340339204), (0.0, 0.0), (30.0 * DEG, -0.340339204)];
/// Original high-wing dihedral estimate, Cl_beta=-0.10/rad.
pub const CL_BETA: &[(f64, f64)] = &[(-30.0 * DEG, 0.052359878), (0.0, 0.0), (30.0 * DEG, -0.052359878)];
/// Original tail-volume directional stability estimate, Cn_beta=0.15/rad.
pub const CN_BETA: &[(f64, f64)] = &[(-30.0 * DEG, -0.078539816), (0.0, 0.0), (30.0 * DEG, 0.078539816)];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Derivatives {
    pub cl_p: f64,
    pub cl_r: f64,
    pub cm_q: f64,
    pub cn_p: f64,
    pub cn_r: f64,
    pub cy_r: f64,
    pub cl_aileron: f64,
    pub cm_elevator: f64,
    pub cn_rudder: f64,
    pub cy_rudder: f64,
    pub cn_aileron: f64,
}
/// Original strip-theory roll damping, tail-volume pitch/yaw damping, and control
/// effectiveness estimates in the conventional nondimensional rate system (pb/2V, qc/2V, rb/2V).
/// Control derivatives are per full normalised travel, not per radian.
pub const DERIVATIVES: Derivatives = Derivatives {
    cl_p: -0.5,
    cl_r: 0.12,
    cm_q: -8.0,
    cn_p: -0.03,
    cn_r: -0.25,
    cy_r: 0.2,
    cl_aileron: 0.16,
    cm_elevator: 1.0,
    cn_rudder: 0.10,
    cy_rudder: 0.18,
    cn_aileron: -0.012,
};

/// Original fixed-pitch prop estimates, J=V/(nD), CT=T/(rho n² D⁴).
/// Smooth unloading to zero thrust at J=1.1, then windmilling drag.
pub const PROP_CT: &[(f64, f64)] = &[
    (0.0, 0.105),
    (0.3, 0.093),
    (0.6, 0.066),
    (0.9, 0.025),
    (1.1, 0.0),
    (1.4, -0.025),
    (3.0, -0.04),
];
/// Original CP=P/(rho n³ D⁵); paired with CT for peak efficiency below 0.85.
pub const PROP_CP: &[(f64, f64)] = &[
    (0.0, 0.055),
    (0.3, 0.052),
    (0.6, 0.047),
    (0.9, 0.038),
    (1.1, 0.030),
    (1.4, 0.022),
    (3.0, 0.022),
];
/// Original normally aspirated engine: available shaft power proportional to density ratio.
pub const POWER_DENSITY: &[(f64, f64)] = &[
    (0.0, 0.0),
    (0.25, 0.25),
    (0.5, 0.5),
    (0.75, 0.75),
    (1.0, 1.0),
    (1.5, 1.5),
];
