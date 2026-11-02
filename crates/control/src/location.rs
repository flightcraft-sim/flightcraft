//! Device location is requested only by explicit command and is never persisted here.
use crate::{
    Command, CommandError, Session,
    navigation::{failed, valid_point},
};
use flightcraft_world::GeoPoint;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CurrentLocation {
    #[serde(default)]
    pub request_permission: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct DeviceLocation {
    pub position: GeoPoint,
    pub accuracy_m: f64,
    pub source: String,
}
impl Command for CurrentLocation {
    const NAME: &'static str = "location.current";
    const DESCRIPTION: &'static str = "Request device location explicitly with request_permission:true; never save the position unless made into a pin.";
    type Output = DeviceLocation;
    fn run(self, s: &mut Session) -> Result<DeviceLocation, CommandError> {
        if !self.request_permission {
            return Err(failed(
                "choose My location or explicitly supply request_permission:true to request OS permission; position stays on device unless saved as a pin",
            ));
        }
        if s.camera.is_some() {
            return Err(CommandError::AppPending(crate::AppRequest::Location));
        }
        #[cfg(target_os = "linux")]
        {
            linux_location()
        }
        #[cfg(target_arch = "wasm32")]
        {
            Err(CommandError::AppPending(crate::AppRequest::Location))
        }
        #[cfg(all(not(target_os = "linux"), not(target_arch = "wasm32")))]
        {
            Err(failed("unavailable on this platform, enter a location"))
        }
    }
}
#[cfg(target_os = "linux")]
fn linux_location() -> Result<DeviceLocation, CommandError> {
    use zbus::{
        blocking::{Proxy, connection::Builder},
        zvariant::OwnedObjectPath,
    };
    let error = |e: zbus::Error| {
        failed(format!(
            "GeoClue location denied or unavailable: {e}; enter a location"
        ))
    };
    let connection = Builder::system()
        .map_err(error)?
        .method_timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(error)?;
    let manager = Proxy::new(
        &connection,
        "org.freedesktop.GeoClue2",
        "/org/freedesktop/GeoClue2/Manager",
        "org.freedesktop.GeoClue2.Manager",
    )
    .map_err(error)?;
    let path: OwnedObjectPath = manager.call("GetClient", &()).map_err(error)?;
    let client = Proxy::new(
        &connection,
        "org.freedesktop.GeoClue2",
        path.as_str(),
        "org.freedesktop.GeoClue2.Client",
    )
    .map_err(error)?;
    client
        .set_property("DesktopId", "flightcraft")
        .map_err(|e| failed(e.to_string()))?;
    client
        .set_property("RequestedAccuracyLevel", 4_u32)
        .map_err(|e| failed(e.to_string()))?;
    client.call::<_, _, ()>("Start", &()).map_err(error)?;
    let work = || -> Result<DeviceLocation, CommandError> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        for _ in 0..40 {
            if std::time::Instant::now() >= deadline {
                break;
            }
            let path: OwnedObjectPath = client.get_property("Location").map_err(error)?;
            if path.as_str() != "/" {
                let location = Proxy::new(
                    &connection,
                    "org.freedesktop.GeoClue2",
                    path.as_str(),
                    "org.freedesktop.GeoClue2.Location",
                )
                .map_err(error)?;
                let r = DeviceLocation {
                    position: GeoPoint {
                        latitude_deg: location.get_property("Latitude").map_err(error)?,
                        longitude_deg: location.get_property("Longitude").map_err(error)?,
                    },
                    accuracy_m: location.get_property("Accuracy").map_err(error)?,
                    source: "GeoClue2".into(),
                };
                valid_point(r.position)?;
                if !r.accuracy_m.is_finite() || r.accuracy_m < 0.0 {
                    return Err(failed("invalid location accuracy"));
                }
                return Ok(r);
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        Err(failed("GeoClue has no location fix; enter a location"))
    };
    let result = work();
    let _ = client.call::<_, _, ()>("Stop", &());
    let _ = manager.call::<_, _, ()>("DeleteClient", &(path.clone(),));
    result
}
#[cfg(target_arch = "wasm32")]
pub async fn location_browser() -> Result<DeviceLocation, CommandError> {
    use wasm_bindgen::{JsCast, closure::Closure};
    let geo = web_sys::window()
        .ok_or_else(|| failed("window unavailable"))?
        .navigator()
        .geolocation()
        .map_err(|_| failed("geolocation unavailable; enter a location"))?;
    let mut success_callback = None;
    let mut error_callback = None;
    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let reject_early = reject.clone();
        success_callback = Some(Closure::wrap(Box::new(move |p: wasm_bindgen::JsValue| {
            let _ = resolve.call1(&wasm_bindgen::JsValue::NULL, &p);
        }) as Box<dyn FnMut(wasm_bindgen::JsValue)>));
        error_callback = Some(Closure::wrap(Box::new(move |e: wasm_bindgen::JsValue| {
            let _ = reject.call1(&wasm_bindgen::JsValue::NULL, &e);
        }) as Box<dyn FnMut(wasm_bindgen::JsValue)>));
        let options = web_sys::PositionOptions::new();
        options.set_timeout(10000);
        options.set_maximum_age(0);
        if let (Some(success), Some(error)) = (&success_callback, &error_callback)
            && let Err(e) = geo.get_current_position_with_error_callback_and_options(
                success.as_ref().unchecked_ref(),
                Some(error.as_ref().unchecked_ref()),
                &options,
            )
        {
            let _ = reject_early.call1(&wasm_bindgen::JsValue::NULL, &e);
        }
    });
    let received = wasm_bindgen_futures::JsFuture::from(promise).await;
    drop(success_callback);
    drop(error_callback);
    let value = received
        .map_err(|_| failed("Location permission denied, timed out, or unavailable; enter a location"))?;
    // The Geolocation success callback supplies the browser's Position interface.
    let p: web_sys::Position = value.unchecked_into();
    let c = p.coords();
    let r = DeviceLocation {
        position: GeoPoint {
            latitude_deg: c.latitude(),
            longitude_deg: c.longitude(),
        },
        accuracy_m: c.accuracy(),
        source: "Browser Geolocation API".into(),
    };
    valid_point(r.position)?;
    if !r.accuracy_m.is_finite() || r.accuracy_m < 0.0 {
        return Err(failed("invalid location accuracy"));
    }
    Ok(r)
}
