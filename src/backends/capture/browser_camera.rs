use async_trait::async_trait;
use js_sys::{Array, Object, Reflect};
use nokhwa_core::buffer::Buffer;
use nokhwa_core::error::NokhwaError;
use nokhwa_core::format_filter::FormatFilter;
use nokhwa_core::frame_format::{FrameFormat, SourceFrameFormat};
use nokhwa_core::traits::{AsyncCaptureTrait, Backend, CaptureTrait};
use nokhwa_core::types::{
    ApiBackend, CameraControl, CameraFormat, CameraIndex, CameraInfo, ControlValueSetter,
    KnownCameraControl, Resolution,
};
use wasm_bindgen_futures::JsFuture;
use std::borrow::Cow;
use std::collections::{HashMap, BTreeMap, HashSet};
use std::future::Future;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{
    CanvasRenderingContext2d, Document, Element, MediaDevices, Navigator, OffscreenCanvas, Window, MediaStream, MediaStreamConstraints, HtmlCanvasElement, MediaDeviceInfo, MediaDeviceKind, MediaStreamTrack,
};


macro_rules! jsv {
    ($value:expr) => {{
        JsValue::from($value)
    }};
}

macro_rules! obj {
    ($(($key:expr, $value:expr)),+ ) => {{
        use js_sys::{Map, Object};
        use wasm_bindgen::JsValue;

        let map = Map::new();
        $(
            map.set(&jsv!($key), &jsv!($value));
        )+
        Object::from(map)
    }};
    ($object:expr, $(($key:expr, $value:expr)),+ ) => {{
        use js_sys::{Map, Object};
        use wasm_bindgen::JsValue;

        let map = Map::new();
        $(
            map.set(&jsv!($key), &jsv!($value));
        )+
        let o = Object::from(map);
        Object::assign(&$object, &o)
    }};
}

fn window() -> Result<Window, NokhwaError> {
    match web_sys::window() {
        Some(win) => Ok(win),
        None => Err(NokhwaError::StructureError {
            structure: "web_sys Window".to_string(),
            error: "None".to_string(),
        }),
    }
}

fn media_devices(navigator: &Navigator) -> Result<MediaDevices, NokhwaError> {
    match navigator.media_devices() {
        Ok(media) => Ok(media),
        Err(why) => Err(NokhwaError::StructureError {
            structure: "MediaDevices".to_string(),
            error: format!("{why:?}"),
        }),
    }
}

fn document(window: &Window) -> Result<Document, NokhwaError> {
    match window.document() {
        Some(doc) => Ok(doc),
        None => Err(NokhwaError::StructureError {
            structure: "web_sys Document".to_string(),
            error: "None".to_string(),
        }),
    }
}

fn document_select_elem(doc: &Document, element: &str) -> Result<Element, NokhwaError> {
    match doc.get_element_by_id(element) {
        Some(elem) => Ok(elem),
        None => {
            return Err(NokhwaError::StructureError {
                structure: format!("Document {element}"),
                error: "None".to_string(),
            })
        }
    }
}

fn element_cast<T: JsCast, U: JsCast>(from: T, name: &str) -> Result<U, NokhwaError> {
    if !from.has_type::<U>() {
        return Err(NokhwaError::StructureError {
            structure: name.to_string(),
            error: "Cannot Cast - No Subtype".to_string(),
        });
    }

    let casted = match from.dyn_into::<U>() {
        Ok(cast) => cast,
        Err(_) => {
            return Err(NokhwaError::StructureError {
                structure: name.to_string(),
                error: "Casting Error".to_string(),
            });
        }
    };
    Ok(casted)
}

fn element_cast_ref<'a, T: JsCast, U: JsCast>(
    from: &'a T,
    name: &'a str,
) -> Result<&'a U, NokhwaError> {
    if !from.has_type::<U>() {
        return Err(NokhwaError::StructureError {
            structure: name.to_string(),
            error: "Cannot Cast - No Subtype".to_string(),
        });
    }

    match from.dyn_ref::<U>() {
        Some(v_e) => Ok(v_e),
        None => Err(NokhwaError::StructureError {
            structure: name.to_string(),
            error: "Cannot Cast".to_string(),
        }),
    }
}

fn create_element(doc: &Document, element: &str) -> Result<Element, NokhwaError> {
    match Document::create_element(doc, element) {
        // ???? thank you intellij
        Ok(new_element) => Ok(new_element),
        Err(why) => Err(NokhwaError::StructureError {
            structure: "Document Video Element".to_string(),
            error: format!("{:?}", why.as_string()),
        }),
    }
}

fn set_autoplay_inline(element: &Element) -> Result<(), NokhwaError> {
    if let Err(why) = element.set_attribute("autoplay", "autoplay") {
        return Err(NokhwaError::SetPropertyError {
            property: "Video-autoplay".to_string(),
            value: "autoplay".to_string(),
            error: format!("{why:?}"),
        });
    }

    if let Err(why) = element.set_attribute("playsinline", "playsinline") {
        return Err(NokhwaError::SetPropertyError {
            property: "Video-playsinline".to_string(),
            value: "playsinline".to_string(),
            error: format!("{why:?}"),
        });
    }

    Ok(())
}

#[derive(Copy, Clone, Hash, Ord, PartialOrd, Eq, PartialEq)]
pub enum JSCameraFacingMode {
    Any,
    Environment,
    User,
    Left,
    Right,
}

#[derive(Copy, Clone, Hash, Ord, PartialOrd, Eq, PartialEq)]
pub enum JSCameraResizeMode {
    Any,
    None,
    CropAndScale,
}

#[derive(Copy, Clone, Hash, Ord, PartialOrd, Eq, PartialEq)]
pub enum JSCameraCanvasType {
    OffScreen(OffscreenCanvas),
    HtmlCanvas(HtmlCanvasElement),
}

#[derive(Copy, Clone, Hash, Ord, PartialOrd, Eq, PartialEq)]
pub enum JSCameraMeteringMode {
    None,
    Manual,
    OneShot, // NIKO WHERE THE FUCK ARE WE
    Continuous,
}

impl AsRef<str> for JSCameraMeteringMode {
    fn as_ref(&self) -> &str {
        match self {
            JSCameraMeteringMode::None => "none",
            JSCameraMeteringMode::Manual => "manual",
            JSCameraMeteringMode::OneShot => "single-shot",
            JSCameraMeteringMode::Continuous => "continuous",
        }
    }
} 

impl Into<JsValue> for JSCameraMeteringMode {
    fn into(self) -> JsValue {
        JsValue::from_str(self.as_ref())
    }
}

/// Quirks:
/// - Regular [`CaptureTrait`] will block, something that is undesired in web applications. Use [`AsyncCaptureTrait`]
/// - [REQUIRES AN UP-TO-DATE BROWSER DUE TO USE OF OFFSCREEN CANVAS.](https://caniuse.com/?search=OffscreenCanvas)
/// - [`SourceFrameFormat`]/[`FrameFormat`] does NOT apply, due to browser non-support. All returned streams will be RGB (autodecoded by browser).
/// - Custom Controls
///     - aspectRatio: 8
///     - facingMode: 16
///     - resizeMode: 32
///     - attachedCanvasMode: 64
///     - pointsOfInterest: 128
///     - exposureTime: 256
///     - colorTemprature: 512
///     - iso: 1024
///     - focusDistance: 2048
///     - zoom: 4096
///     - torch: 8192
pub struct BrowserCamera {
    index: CameraIndex,
    info: CameraInfo,
    format: CameraFormat,
    media_stream: MediaStream,
    init: bool,
    custom_controls: HashMap<u128, CameraControl>,
    controls: HashMap<KnownCameraControl, CameraControl>,
    supported_controls: HashSet<KnownCameraControl>,
    cavnas: Option<CanvasType>,
    context: Option<CanvasRenderingContext2d>,
}

impl BrowserCamera {
    pub fn new(index: &CameraIndex) -> Result<BrowserCamera, NokhwaError> {
        wasm_rs_async_executor::single_threaded::block_on(Self::new_async(index))
    }

    pub async fn new_async(index: &CameraIndex) -> Result<BrowserCamera, NokhwaError> {
        let window = window()?;
        let media_devices = media_devices(&window.navigator())?;

        let stream: MediaStream = match media_devices.get_user_media_with_constraints(&constraints)
        {
            Ok(promise) => {
                let future = JsFuture::from(promise);
                match future.await {
                    Ok(stream) => {
                        let media_stream: MediaStream = MediaStream::from(stream);
                        media_stream
                    }
                    Err(why) => {
                        return Err(NokhwaError::OpenDeviceError(
                            "MediaDevicesGetUserMediaJsFuture".to_string(), format!("{why:?}"),
                        ))
                    }
                }
            }
            Err(why) => {
                return Err(NokhwaError::OpenDeviceError(
                    "MediaDevicesGetUserMedia".to_string(), format!("{why:?}"),
                ))
            }
        };

        let media_info = match media_devices.enumerate_devices() {
            Ok(i) => {
                let future = JsFuture::from(promise);
                match future.await {
                    Ok(devs) => {
                        let arr = Array::from(&devs);
                        match index {
                            CameraIndex::Index(i) => {
                                let dr = arr.get(i as u32);

                                if dr == JsValue::UNDEFINED {
                                    return Err(NokhwaError::StructureError { structure: "MediaDeviceInfo".to_string(), error: "undefined".to_string() })
                                }

                                MediaDeviceInfo::from(dr)
                            }
                            CameraIndex::String(s) => {
                                match arr.iter().map(MediaDeviceInfo::from)
                                .filter(|mdi| {
                                    mdi.device_id() == s
                                }).nth(0) {
                                    Some(i) => i,
                                    None => return Err(NokhwaError::StructureError { structure: "MediaDeviceInfo".to_string(), error: "no id".to_string() })

                                }
                            }
                        }
                    }
                    Err(why) => {
                        return Err(NokhwaError::StructureError { structure: "MediaDeviceInfo Enumerate Devices Promise".to_string(), error: format!("{why:?}") })
                    }
                }
            }
            Err(why) => {
                return Err(NokhwaError::GetPropertyError { property: "MediaDeviceInfo".to_string(), error: format!("{why:?}") })
            },
        };

        let info = CameraInfo::new(media_info.label(), media_info.kind().to_string(), format!("{}:{}", media_info.group_id().to_string(), media_info.device_id().to_string()),index.clone(),);

        Ok(BrowserCamera { index:  index.clone(), info, format: CameraFormat::default(), init: false, cavnas: None, context: None, media_stream: stream, controls: HashMap::new(), custom_controls: HashMap::new(), supported_controls: HashSet::new() })
    }
}

impl Backend for BrowserCamera {
    const BACKEND: ApiBackend = ApiBackend::Browser;
}

impl CaptureTrait for BrowserCamera {
    fn init(&mut self) -> Result<(), NokhwaError> {

    }

    fn init_with_format(&mut self, format: FormatFilter) -> Result<CameraFormat, NokhwaError> {
        self.init()?;
    }

    fn backend(&self) -> ApiBackend {
        todo!()
    }

    fn camera_info(&self) -> &CameraInfo {
        todo!()
    }

    fn refresh_camera_format(&mut self) -> Result<(), NokhwaError> {
        todo!()
    }

    fn camera_format(&self) -> Option<CameraFormat> {
        todo!()
    }

    fn set_camera_format(&mut self, new_fmt: CameraFormat) -> Result<(), NokhwaError> {
        todo!()
    }

    fn compatible_list_by_resolution(
        &mut self,
        fourcc: SourceFrameFormat,
    ) -> Result<HashMap<Resolution, Vec<u32>>, NokhwaError> {
        todo!()
    }

    fn compatible_fourcc(&mut self) -> Result<Vec<SourceFrameFormat>, NokhwaError> {
        todo!()
    }

    fn resolution(&self) -> Option<Resolution> {
        todo!()
    }

    fn set_resolution(&mut self, new_res: Resolution) -> Result<(), NokhwaError> {
        todo!()
    }

    fn frame_rate(&self) -> Option<u32> {
        todo!()
    }

    fn set_frame_rate(&mut self, new_fps: u32) -> Result<(), NokhwaError> {
        todo!()
    }

    fn frame_format(&self) -> SourceFrameFormat {
        todo!()
    }

    fn set_frame_format(
        &mut self,
        fourcc: impl Into<SourceFrameFormat>,
    ) -> Result<(), NokhwaError> {
        todo!()
    }

    fn camera_control(&self, control: KnownCameraControl) -> Result<CameraControl, NokhwaError> {
        // controls

        if self.supported_controls.get(&control).is_none() {
            return Err(NokhwaError::GetPropertyError { property: control.to_string(), error: "Not Supported".to_string() });
        }

        // get the data!

        let track =self.media_stream.get_video_tracks().get(0);

        if track.is_undefined() || track.is_null() {
            return Err(NokhwaError::GetPropertyError { property: "MediaStreamTrack".to_string(), error: "Null".to_string() });
        }

        let track = MediaStreamTrack::from(track);
        track.
    }

    fn camera_controls(&self) -> Result<Vec<CameraControl>, NokhwaError> {
        // controls
    }

    fn set_camera_control(
        &mut self,
        id: KnownCameraControl,
        value: ControlValueSetter,
    ) -> Result<(), NokhwaError> {
        todo!()
    }

    fn open_stream(&mut self) -> Result<(), NokhwaError> {
        todo!()
    }

    fn is_stream_open(&self) -> bool {
        todo!()
    }

    fn frame(&mut self) -> Result<Buffer, NokhwaError> {
        todo!()
    }

    fn frame_raw(&mut self) -> Result<Cow<[u8]>, NokhwaError> {
        todo!()
    }

    fn stop_stream(&mut self) -> Result<(), NokhwaError> {
        todo!()
    }
}

#[cfg(feature = "output-async")]
impl AsyncCaptureTrait for BrowserCamera {
    async fn init_async(&mut self) -> Result<(), NokhwaError> {
        let window = window()?;
        let media_devices = media_devices(&window.navigator())?;

        // request permission for camera
         

        // first populate supported controls and see if we have our required controls
        // required: FPS, Resolution (width + height)
        // everything else is optional (whiteBalanceMode, exposureMode, focusMode, pointsOfInterest, exposureCompensation, exposureTime, colorTemprature, iso, brightness, contrast, pan, saturation, sharpness, focusDistance, tilt, zoom, torch)

        let browser_constraints = media_devices.get_supported_constraints();

        let mut supported_constraints = HashSet::new();

        let defaults_satisfied = {
            Reflect::get(&browser_constraints, "frameRate".into()).map(|x| x.is_truthy()).unwrap_or(false) && Reflect::get(&browser_constraints, "width".into()).map(|x| x.is_truthy()).unwrap_or(false) && Reflect::get(&browser_constraints, "height".into()).map(|x| x.is_truthy()).unwrap_or(false)
        };

        // STAY ~~WHITE~~ CLEAN WITH US! JOIN ~~WHITE~~ MATCH EXPRESSION SOCIETY!

        // aspectRatio
        if Reflect::get(&browser_constraints, "aspectRatio".into()).map(|x| x.is_truthy()).unwrap_or(false) {
            supported_constraints.insert(KnownCameraControl::Other(8));
        }

        // facingMode
        if Reflect::get(&browser_constraints, "facingMode".into()).map(|x| x.is_truthy()).unwrap_or(false) {
            supported_constraints.insert(KnownCameraControl::Other(16));
        }

        // resizeMode
        if Reflect::get(&browser_constraints, "resizeMode".into()).map(|x| x.is_truthy()).unwrap_or(false) {
            supported_constraints.insert(KnownCameraControl::Other(32));
        }

        // attachedCanvasMode
        supported_constraints.insert(KnownCameraControl::Other(64));

        // whiteBalanceMode
        if Reflect::get(&browser_constraints, "whiteBalanceMode".into()).map(|x| x.is_truthy()).unwrap_or(false) {
            supported_constraints.insert(KnownCameraControl::WhiteBalance);
        }

        // exposureMode
        if Reflect::get(&browser_constraints, "exposureMode".into()).map(|x| x.is_truthy()).unwrap_or(false) {
            supported_constraints.insert(KnownCameraControl::Exposure);
        }

        // focusMode
        if Reflect::get(&browser_constraints, "focusMode".into()).map(|x| x.is_truthy()).unwrap_or(false) {
            supported_constraints.insert(KnownCameraControl::Focus);
        }

        // pointsOfInterest
        if Reflect::get(&browser_constraints, "pointsOfInterest".into()).map(|x| x.is_truthy()).unwrap_or(false) {
            supported_constraints.insert(KnownCameraControl::Other(128));
        }

        // exposureCompensation
        if Reflect::get(&browser_constraints, "exposureCompensation".into()).map(|x| x.is_truthy()).unwrap_or(false) {
            supported_constraints.insert(KnownCameraControl::Exposure);
        }

        // exposureTime
        if Reflect::get(&browser_constraints, "exposureTime".into()).map(|x| x.is_truthy()).unwrap_or(false) {
            supported_constraints.insert(KnownCameraControl::Other(256));
        }

        // colorTemprature
        if Reflect::get(&browser_constraints, "colorTemprature".into()).map(|x| x.is_truthy()).unwrap_or(false) {
            supported_constraints.insert(KnownCameraControl::Other(512));
        }

        // iso
        if Reflect::get(&browser_constraints, "iso".into()).map(|x| x.is_truthy()).unwrap_or(false) {
            supported_constraints.insert(KnownCameraControl::Other(1024));
        }

        // brightness
        if Reflect::get(&browser_constraints, "brightness".into()).map(|x| x.is_truthy()).unwrap_or(false) {
            supported_constraints.insert(KnownCameraControl::Brightness);
        }

        // contrast
        if Reflect::get(&browser_constraints, "contrast".into()).map(|x| x.is_truthy()).unwrap_or(false) {
            supported_constraints.insert(KnownCameraControl::Contrast);
        }

        // pan
        if Reflect::get(&browser_constraints, "pan".into()).map(|x| x.is_truthy()).unwrap_or(false) {
            supported_constraints.insert(KnownCameraControl::Pan);
        }

        // saturation
        if Reflect::get(&browser_constraints, "saturation".into()).map(|x| x.is_truthy()).unwrap_or(false) {
            supported_constraints.insert(KnownCameraControl::Saturation);
        }

        // sharpness
        if Reflect::get(&browser_constraints, "sharpness".into()).map(|x| x.is_truthy()).unwrap_or(false) {
            supported_constraints.insert(KnownCameraControl::Sharpness);
        }

        // focusDistance
        if Reflect::get(&browser_constraints, "focusDistance".into()).map(|x| x.is_truthy()).unwrap_or(false) {
            supported_constraints.insert(KnownCameraControl::Other(2048));
        }

        // tilt
        if Reflect::get(&browser_constraints, "tilt".into()).map(|x| x.is_truthy()).unwrap_or(false) {
            supported_constraints.insert(KnownCameraControl::Tilt);
        }

        // zoom
        if Reflect::get(&browser_constraints, "zoom".into()).map(|x| x.is_truthy()).unwrap_or(false) {
            supported_constraints.insert(KnownCameraControl::Other(4096));
        }

        // torch
        if Reflect::get(&browser_constraints, "torch".into()).map(|x| x.is_truthy()).unwrap_or(false) {
            supported_constraints.insert(KnownCameraControl::Other(8192));
        }

        // PUT ME INTO THE CHARLOTTE VESSEL COACH I'LL PROVE FREE WILL IS REAL

        if supported_constraints.contains(&KnownCameraControl::Pan) || supported_constraints.contains(&KnownCameraControl::Tilt) || supported_constraints.contains(&KnownCameraControl::Other(4096)) {
            // TODO: Zoom pan and Tilt MUST BE REQUESTED HERE!!
        }

        self.supported_controls = supported_constraints;

        // get values for supported controls

        for control in self.supported_controls {
            match control {
                KnownCameraControl::Brightness => {
                    
                }
                KnownCameraControl::Contrast => todo!(),
                KnownCameraControl::Hue => todo!(),
                KnownCameraControl::Saturation => todo!(),
                KnownCameraControl::Sharpness => todo!(),
                KnownCameraControl::Gamma => todo!(),
                KnownCameraControl::WhiteBalance => todo!(),
                KnownCameraControl::BacklightComp => todo!(),
                KnownCameraControl::Gain => todo!(),
                KnownCameraControl::Pan => todo!(),
                KnownCameraControl::Tilt => todo!(),
                KnownCameraControl::Zoom => todo!(),
                KnownCameraControl::Exposure => todo!(),
                KnownCameraControl::Iris => todo!(),
                KnownCameraControl::Focus => todo!(),
                KnownCameraControl::Other(_) => todo!(),

            }
        }

        todo!()
    }

    async fn init_with_format_async(&mut self, format: FormatFilter) -> Result<CameraFormat, NokhwaError> {
        todo!()
    }

    async fn refresh_camera_format_async(&mut self) -> Result<(), NokhwaError> {
        todo!()
    }

    async fn set_camera_format_async(&mut self, new_fmt: CameraFormat) -> Result<(), NokhwaError> {
        todo!()
    }

    async fn compatible_list_by_resolution_async(&mut self, fourcc: SourceFrameFormat) -> Result<HashMap<Resolution, Vec<u32>>, NokhwaError> {
        todo!()
    }

    async fn compatible_camera_formats_async(&mut self) -> Result<Vec<CameraFormat>, NokhwaError> {
        todo!()
    }

    async fn compatible_fourcc_async(&mut self) -> Result<Vec<SourceFrameFormat>, NokhwaError> {
        todo!()
    }

    async fn set_resolution_async(&mut self, new_res: Resolution) -> Result<(), NokhwaError> {
        todo!()
    }

    async fn set_frame_rate_async(&mut self, new_fps: u32) -> Result<(), NokhwaError> {
        todo!()
    }

    async fn set_frame_format_async(&mut self, fourcc: SourceFrameFormat) -> Result<(), NokhwaError> {
        todo!()
    }

    async fn camera_control_async(&self, control: KnownCameraControl) -> Result<CameraControl, NokhwaError> {
        todo!()
    }

    async fn camera_controls_async(&self) -> Result<Vec<CameraControl>, NokhwaError> {
        todo!()
    }

    async fn set_camera_control_async(&mut self, id: KnownCameraControl, value: ControlValueSetter) -> Result<(), NokhwaError> {
        todo!()
    }

    async fn open_stream_async(&mut self) -> Result<(), NokhwaError> {
        todo!()
    }

    async fn frame_async(&mut self) -> Result<Buffer, NokhwaError> {
        todo!()
    }

    async fn frame_raw_async(&mut self) -> Result<Cow<[u8]>, NokhwaError> {
        todo!()
    }

    async fn stop_stream_async(&mut self) -> Result<(), NokhwaError> {
        todo!()
    }
}
