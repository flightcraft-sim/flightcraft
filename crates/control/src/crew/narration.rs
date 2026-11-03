use super::*;
impl Crew {
    pub(super) fn narrate_landmarks(
        &mut self,
        s: &mut Session,
        phase: PilotPhase,
        [n, e, _]: [f64; 3],
        heading: f64,
        now: f64,
    ) {
        if !matches!(phase, PilotPhase::Cruise | PilotPhase::Descent)
            || now - self.last_speech < 20.
            || self.last_landmark_s.is_some_and(|last| now - last < 120.)
        {
            return;
        }
        if now - self.last_landmark_check < 20. {
            return;
        }
        self.last_landmark_check = now;
        let Some(ownship) = planning::geographic(s, [n, e, 0.]) else {
            return;
        };
        let Some(data) = s.sim.world().landmarks() else {
            return;
        };
        let mut candidates: Vec<_> = data
            .features
            .iter()
            .filter(|f| {
                f.prominence.is_some()
                    && !self.spoken_landmarks.contains(&f.feature_id)
                    && (f.position.latitude_deg - ownship.latitude_deg).abs() < 0.12
                    && (f.position.longitude_deg - ownship.longitude_deg).abs() < 0.16
            })
            .filter_map(|f| {
                s.radio_project(f.position.latitude_deg, f.position.longitude_deg)
                    .map(|[fnorth, feast]| {
                        let dn = fnorth - n;
                        let de = feast - e;
                        let angle = flightcraft_sim::autopilot::wrap(libm::atan2(de, dn) - heading);
                        let distance = libm::hypot(dn, de);
                        let priority = f.prominence.unwrap_or(4);
                        (priority, distance, angle, f)
                    })
            })
            .filter(|(_, d, a, _)| *d < 6. * 1852. && *d > 150. && a.abs() < 2.3)
            .collect();
        candidates.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1.total_cmp(&b.1))
                .then_with(|| a.3.feature_id.cmp(&b.3.feature_id))
        });
        if let Some((_, _, angle, feature)) = candidates.first() {
            let mut text = format!("That's {}, {}.", spoken_name(feature), relative_position(*angle));
            self.spoken_landmarks.insert(feature.feature_id);
            if let Some(description) = &feature.description {
                text.push(' ');
                text.push_str(description);
            }
            s.crew_say(text);
            self.last_speech = now;
            self.last_landmark_s = Some(now);
        }
    }
}
fn spoken_name(feature: &flightcraft_world::Landmark) -> String {
    if matches!(feature.feature_class.as_str(), "Gap" | "Stream" | "Ridge")
        && !feature.name.to_lowercase().starts_with("the ")
    {
        format!("the {}", feature.name)
    } else {
        feature.name.clone()
    }
}
fn relative_position(angle: f64) -> String {
    let degrees = angle.to_degrees();
    if degrees.abs() < 15. {
        return "straight ahead".into();
    }
    if (70. ..=110.).contains(&degrees.abs()) {
        return format!("off the {} wing", if degrees < 0. { "left" } else { "right" });
    }
    let clock = ((degrees.rem_euclid(360.) / 30.).round() as usize) % 12;
    let word = [
        "twelve", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven",
    ]
    .get(clock)
    .copied()
    .unwrap_or("twelve");
    format!("at our {word} o'clock")
}
