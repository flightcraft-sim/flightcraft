//! Aircraft definitions: geometry, mass, coefficient tables, and propulsion.
//! Pure functions from an airflow state to forces and moments. No UI, no integration.
//!
//! Deterministic: transcendental maths goes through `libm`, never `std` float methods.
