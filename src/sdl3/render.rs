//! 2D accelerated rendering
//!
//! Official C documentation: https://wiki.libsdl.org/CategoryRender
//! # Introduction
//!
//! This module contains functions for 2D accelerated rendering.
//!
//! This API supports the following features:
//!
//! * single pixel points
//! * single pixel lines
//! * filled rectangles
//! * texture images
//! * All of these may be drawn in opaque, blended, or additive modes.
//!
//! The texture images can have an additional color tint or alpha modulation
//! applied to them, and may also be stretched with linear interpolation,
//! rotated or flipped/mirrored.
//!
//! For advanced functionality like particle effects or actual 3D you should use
//! SDL's OpenGL/Direct3D support or one of the many available 3D engines.
//!
//! This API is not designed to be used from multiple threads, see
//! [this bug](http://bugzilla.libsdl.org/show_bug.cgi?id=1995) for details.
//!
//! ---
//!
//! None of the draw methods in `Canvas` are expected to fail.
//! If they do, a panic is raised and the program is aborted.

use crate::common::{validate_int, IntegerOrSdlError};
use crate::get_error;
use crate::pixels;
use crate::rect::Point;
use crate::rect::Rect;
use crate::surface::{Surface, SurfaceContext, SurfaceRef};
use crate::sys;
use crate::video::{Window, WindowContext};
use crate::Error;
use libc::{c_double, c_int};
use pixels::PixelFormat;
use std::convert::{Into, TryFrom, TryInto};
use std::error;
use std::ffi::CStr;
use std::fmt;
#[cfg(not(feature = "unsafe_textures"))]
use std::marker::PhantomData;
use std::mem;
use std::mem::{transmute, MaybeUninit};
use std::ops::Deref;
use std::ptr;
use std::rc::Rc;
use std::sync::Arc;
use sys::blendmode::SDL_BlendMode;
use sys::everything::SDL_PropertiesID;
use sys::render::{SDL_GetTextureProperties, SDL_TextureAccess};
use sys::stdinc::Sint64;
use sys::surface::{SDL_FLIP_HORIZONTAL, SDL_FLIP_NONE, SDL_FLIP_VERTICAL};

/// Possible errors returned by targeting a `Canvas` to render to a `Texture`
#[derive(Debug, Clone)]
pub enum TargetRenderError {
    SdlError(Error),
}

impl fmt::Display for TargetRenderError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::TargetRenderError::*;
        match *self {
            SdlError(ref e) => write!(f, "SDL error: {}", e),
        }
    }
}

impl error::Error for TargetRenderError {
    fn description(&self) -> &str {
        use self::TargetRenderError::*;
        match self {
            SdlError(e) => &e.0,
        }
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
#[repr(i32)]
pub enum TextureAccess {
    Static = sys::render::SDL_TEXTUREACCESS_STATIC.0,
    Streaming = sys::render::SDL_TEXTUREACCESS_STREAMING.0,
    Target = sys::render::SDL_TEXTUREACCESS_TARGET.0,
}

impl From<TextureAccess> for sys::render::SDL_TextureAccess {
    fn from(access: TextureAccess) -> sys::render::SDL_TextureAccess {
        sys::render::SDL_TextureAccess(access as i32)
    }
}

impl From<SDL_TextureAccess> for TextureAccess {
    fn from(access: SDL_TextureAccess) -> TextureAccess {
        match access {
            sys::render::SDL_TEXTUREACCESS_STATIC => TextureAccess::Static,
            sys::render::SDL_TEXTUREACCESS_STREAMING => TextureAccess::Streaming,
            sys::render::SDL_TEXTUREACCESS_TARGET => TextureAccess::Target,
            _ => panic!("Unknown texture access value: {}", access.0),
        }
    }
}

impl From<i64> for TextureAccess {
    fn from(n: i64) -> TextureAccess {
        let texture_access_c_int: std::ffi::c_int = n
            .try_into()
            .expect("Pixel format value out of range for c_int");
        let texture_access = SDL_TextureAccess(texture_access_c_int);
        texture_access.into()
    }
}

// floating-point point
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct FPoint {
    pub x: f32,
    pub y: f32,
}
impl FPoint {
    pub fn new(x: f32, y: f32) -> FPoint {
        FPoint { x, y }
    }
    pub fn to_ll(&self) -> sys::rect::SDL_FPoint {
        sys::rect::SDL_FPoint {
            x: self.x,
            y: self.y,
        }
    }
}

impl From<Point> for FPoint {
    fn from(point: Point) -> Self {
        FPoint::new(point.x as f32, point.y as f32)
    }
}

impl From<(f32, f32)> for FPoint {
    fn from(point: (f32, f32)) -> Self {
        FPoint::new(point.0, point.1)
    }
}

impl From<(i32, i32)> for FPoint {
    fn from(point: (i32, i32)) -> Self {
        FPoint::new(point.0 as f32, point.1 as f32)
    }
}

impl From<(u32, u32)> for FPoint {
    fn from(point: (u32, u32)) -> Self {
        FPoint::new(point.0 as f32, point.1 as f32)
    }
}

// floating-point rectangle
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct FRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
impl FRect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> FRect {
        FRect { x, y, w, h }
    }
    pub fn to_ll(&self) -> sys::rect::SDL_FRect {
        sys::rect::SDL_FRect {
            x: self.x,
            y: self.y,
            w: self.w,
            h: self.h,
        }
    }
    pub fn set_x(&mut self, update: f32) {
        self.x = update;
    }
    pub fn set_y(&mut self, update: f32) {
        self.y = update;
    }
    pub fn set_w(&mut self, update: f32) {
        self.w = update;
    }
    pub fn set_h(&mut self, update: f32) {
        self.h = update;
    }
    pub fn set_xy(&mut self, update: FPoint) {
        self.x = update.x;
        self.y = update.y;
    }
}

impl From<Rect> for FRect {
    fn from(rect: Rect) -> Self {
        FRect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32)
    }
}

impl From<(i32, i32, u32, u32)> for FRect {
    fn from(rect: (i32, i32, u32, u32)) -> Self {
        FRect::new(rect.0 as f32, rect.1 as f32, rect.2 as f32, rect.3 as f32)
    }
}

#[derive(Debug)]
pub struct InvalidTextureAccess(u32);

impl std::fmt::Display for InvalidTextureAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Invalid texture access value: {}", self.0)
    }
}

impl std::error::Error for InvalidTextureAccess {}

impl TryFrom<u32> for TextureAccess {
    type Error = InvalidTextureAccess;

    fn try_from(n: u32) -> Result<Self, Self::Error> {
        // Convert the u32 to SDL_TextureAccess
        let sdl_access = SDL_TextureAccess(n as i32);

        // Match against the SDL_TextureAccess constants
        if sdl_access == SDL_TextureAccess::STATIC {
            Ok(TextureAccess::Static)
        } else if sdl_access == SDL_TextureAccess::STREAMING {
            Ok(TextureAccess::Streaming)
        } else if sdl_access == SDL_TextureAccess::TARGET {
            Ok(TextureAccess::Target)
        } else {
            Err(InvalidTextureAccess(n))
        }
    }
}

/// A structure that contains information on the capabilities of a render driver
/// or the current render context.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct RendererInfo {
    pub name: &'static str,
    pub flags: u32,
    pub texture_formats: Vec<PixelFormat>,
    pub max_texture_width: u32,
    pub max_texture_height: u32,
}

/// Blend mode for `Canvas`, `Texture` or `Surface`.
#[repr(i32)]
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum BlendMode {
    /// no blending (replace destination with source).
    None = sys::blendmode::SDL_BLENDMODE_NONE as i32,
    /// Alpha blending
    ///
    /// dstRGB = (srcRGB * srcA) + (dstRGB * (1-srcA))
    ///
    /// dstA = srcA + (dstA * (1-srcA))
    Blend = sys::blendmode::SDL_BLENDMODE_BLEND as i32,
    /// Additive blending
    ///
    /// dstRGB = (srcRGB * srcA) + dstRGB
    ///
    /// dstA = dstA (keep original alpha)
    Add = sys::blendmode::SDL_BLENDMODE_ADD as i32,
    /// Color modulate
    ///
    /// dstRGB = srcRGB * dstRGB
    Mod = sys::blendmode::SDL_BLENDMODE_MOD as i32,
    /// Color multiply
    Mul = sys::blendmode::SDL_BLENDMODE_MUL as i32,
    /// Invalid blending mode (indicates error)
    Invalid = sys::blendmode::SDL_BLENDMODE_INVALID as i32,
}

impl TryFrom<u32> for BlendMode {
    type Error = ();

    fn try_from(n: u32) -> Result<Self, Self::Error> {
        use self::BlendMode::*;

        Ok(match n {
            sys::blendmode::SDL_BLENDMODE_NONE => None,
            sys::blendmode::SDL_BLENDMODE_BLEND => Blend,
            sys::blendmode::SDL_BLENDMODE_ADD => Add,
            sys::blendmode::SDL_BLENDMODE_MOD => Mod,
            sys::blendmode::SDL_BLENDMODE_MUL => Mul,
            sys::blendmode::SDL_BLENDMODE_INVALID => Invalid,
            _ => return Err(()),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClippingRect {
    /// a non-zero area clipping rect
    Some(Rect),
    /// a clipping rect with zero area
    Zero,
    /// the absence of a clipping rect
    None,
}

impl Into<ClippingRect> for Rect {
    fn into(self) -> ClippingRect {
        ClippingRect::Some(self)
    }
}

impl Into<ClippingRect> for Option<Rect> {
    fn into(self) -> ClippingRect {
        match self {
            Some(v) => v.into(),
            None => ClippingRect::None,
        }
    }
}

impl ClippingRect {
    pub fn intersection(&self, other: ClippingRect) -> ClippingRect {
        match self {
            ClippingRect::Zero => ClippingRect::Zero,
            ClippingRect::None => other,
            ClippingRect::Some(self_rect) => match other {
                ClippingRect::Zero => ClippingRect::Zero,
                ClippingRect::None => *self,
                ClippingRect::Some(rect) => match self_rect.intersection(rect) {
                    Some(v) => ClippingRect::Some(v),
                    None => ClippingRect::Zero,
                },
            },
        }
    }

    /// shrink the clipping rect to the part which contains the position
    pub fn intersect_rect<R>(&self, position: R) -> ClippingRect
    where
        R: Into<Option<Rect>>,
    {
        let position: Option<Rect> = position.into();
        match position {
            Some(position) => {
                match self {
                    ClippingRect::Some(rect) => match rect.intersection(position) {
                        Some(v) => ClippingRect::Some(v),
                        None => ClippingRect::Zero,
                    },
                    ClippingRect::Zero => ClippingRect::Zero,
                    ClippingRect::None => {
                        // clipping rect has infinite area, so it's just whatever position is
                        ClippingRect::Some(position)
                    }
                }
            }
            None => {
                // position is zero area so intersection result is zero
                ClippingRect::Zero
            }
        }
    }
}

/// Manages what keeps a `SDL_Renderer` alive
///
/// When the `RendererContext` is dropped, it destroys the `SDL_Renderer`
pub struct RendererContext<T> {
    raw: *mut sys::render::SDL_Renderer,
    _target: Arc<T>,
}

impl<T> Drop for RendererContext<T> {
    #[doc(alias = "SDL_DestroyRenderer")]
    fn drop(&mut self) {
        unsafe {
            sys::render::SDL_DestroyRenderer(self.raw);
        };
    }
}

impl<T> RendererContext<T> {
    /// Gets the raw pointer to the SDL_Renderer
    // this can prevent introducing UB until
    // https://github.com/rust-lang/rust-clippy/issues/5953 is fixed
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub fn raw(&self) -> *mut sys::render::SDL_Renderer {
        self.raw
    }

    pub unsafe fn from_ll(raw: *mut sys::render::SDL_Renderer, target: Arc<T>) -> Self {
        RendererContext {
            raw,
            _target: target,
        }
    }

    unsafe fn set_raw_target(
        &self,
        raw_texture: *mut sys::render::SDL_Texture,
    ) -> Result<(), Error> {
        if sys::render::SDL_SetRenderTarget(self.raw, raw_texture) {
            Ok(())
        } else {
            Err(get_error())
        }
    }

    unsafe fn get_raw_target(&self) -> *mut sys::render::SDL_Texture {
        sys::render::SDL_GetRenderTarget(self.raw)
    }
}

impl<T: RenderTarget> Deref for Canvas<T> {
    type Target = RendererContext<T::Context>;

    fn deref(&self) -> &RendererContext<T::Context> {
        self.context.as_ref()
    }
}

/// Represents structs which can be the target of a `SDL_Renderer` (or Canvas).
///
/// This is intended for internal use only. It should not be used outside of this crate,
/// but is still visible for documentation reasons.
pub trait RenderTarget {
    type Context;
}

impl<'s> RenderTarget for Surface<'s> {
    type Context = SurfaceContext<'s>;
}

/// Manages and owns a target (`Surface` or `Window`) and allows drawing in it.
///
/// If the `Window` manipulates the shell of the Window, `Canvas<Window>` allows you to
/// manipulate both the shell and the inside of the window;
/// you can manipulate pixel by pixel (*not recommended*), lines, colored rectangles, or paste
/// `Texture`s to this `Canvas`.
///
/// Drawing to the `Canvas` does not take effect immediately, it draws to a buffer until you
/// call `present()`, where all the operations you did until the last `present()`
/// are updated to your target
///
/// Its context may be shared with the `TextureCreator`.
///
/// The context will not be dropped until all references of it are out of scope.
///
/// # Examples
///
/// ```rust,no_run
/// # use sdl3::render::Canvas;
/// # use sdl3::video::Window;
/// # use sdl3::pixels::Color;
/// # use sdl3::rect::Rect;
/// # let sdl_context = sdl3::init().unwrap();
/// # let video_subsystem = sdl_context.video().unwrap();
/// let window = video_subsystem.window("Example", 800, 600).build().unwrap();
///
/// // Let's create a Canvas which we will use to draw in our Window
/// let mut canvas : Canvas<Window> = window.into_canvas();
///
/// canvas.set_draw_color(Color::RGB(0, 0, 0));
/// // fills the canvas with the color we set in `set_draw_color`.
/// canvas.clear();
///
/// // change the color of our drawing with a gold-color ...
/// canvas.set_draw_color(Color::RGB(255, 210, 0));
/// // A draw a rectangle which almost fills our window with it !
/// canvas.fill_rect(Rect::new(10, 10, 780, 580));
///
/// // However the canvas has not been updated to the window yet,
/// // everything has been processed to an internal buffer,
/// // but if we want our buffer to be displayed on the window,
/// // we need to call `present`. We need to call this every time
/// // we want to render a new frame on the window.
/// canvas.present();
/// // present does not "clear" the buffer, that means that
/// // you have to clear it yourself before rendering again,
/// // otherwise leftovers of what you've renderer before might
/// // show up on the window !
/// //
/// // A good rule of thumb is to `clear()`, draw every texture
/// // needed, and then `present()`; repeat this every new frame.
///
/// ```
pub struct Canvas<T: RenderTarget> {
    target: T,
    context: Rc<RendererContext<T::Context>>,
    default_pixel_format: PixelFormat,
    pub renderer_name: String,
}

/// Alias for a `Canvas` that was created out of a `Surface`
pub type SurfaceCanvas<'s> = Canvas<Surface<'s>>;

/// Methods for the `SurfaceCanvas`.
impl<'s> Canvas<Surface<'s>> {
    /// Creates a 2D software rendering context for a surface.
    ///
    /// This method should only fail if SDL2 is not built with rendering
    /// support, or there's an out-of-memory error.
    #[doc(alias = "SDL_CreateSoftwareRenderer")]
    pub fn from_surface(surface: Surface<'s>) -> Result<Self, Error> {
        let raw_renderer = unsafe { sys::render::SDL_CreateSoftwareRenderer(surface.raw()) };
        if !raw_renderer.is_null() {
            let context =
                Rc::new(unsafe { RendererContext::from_ll(raw_renderer, surface.context()) });
            let default_pixel_format = surface.pixel_format_enum();
            Ok(Canvas {
                target: surface,
                context,
                default_pixel_format,
                renderer_name: unsafe {
                    CStr::from_ptr(sys::render::SDL_GetRendererName(raw_renderer))
                        .to_string_lossy()
                        .into_owned()
                },
            })
        } else {
            Err(get_error())
        }
    }

    /// Gets a reference to the associated surface of the Canvas
    #[inline]
    pub fn surface(&self) -> &SurfaceRef {
        &self.target
    }

    /// Gets a mutable reference to the associated surface of the Canvas
    #[inline]
    pub fn surface_mut(&mut self) -> &mut SurfaceRef {
        &mut self.target
    }

    /// Gets the associated surface of the Canvas and destroys the Canvas
    #[inline]
    pub fn into_surface(self) -> Surface<'s> {
        self.target
    }

    /// Returns a `TextureCreator` that can create Textures to be drawn on this `Canvas`
    ///
    /// This `TextureCreator` will share a reference to the renderer and target context.
    ///
    /// The target (i.e., `Window`) will not be destroyed and the SDL_Renderer will not be
    /// destroyed if the `TextureCreator` is still in scope.
    pub fn texture_creator(&self) -> TextureCreator<SurfaceContext<'s>> {
        TextureCreator {
            context: self.context.clone(),
            default_pixel_format: self.default_pixel_format,
        }
    }
}

pub type WindowCanvas = Canvas<Window>;

impl RenderTarget for Window {
    type Context = WindowContext;
}

/// Methods for the `WindowCanvas`.
impl Canvas<Window> {
    /// Gets a reference to the associated window of the Canvas
    #[inline]
    pub fn window(&self) -> &Window {
        &self.target
    }

    /// Gets a mutable reference to the associated window of the Canvas
    #[inline]
    pub fn window_mut(&mut self) -> &mut Window {
        &mut self.target
    }

    /// Gets the associated window of the Canvas and destroys the Canvas
    #[inline]
    pub fn into_window(self) -> Window {
        self.target
    }

    #[inline]
    pub fn default_pixel_format(&self) -> PixelFormat {
        self.window().window_pixel_format()
    }

    pub fn from_window_and_renderer(
        window: Window,
        renderer: *mut sys::render::SDL_Renderer,
    ) -> Self {
        let context = Rc::new(unsafe { RendererContext::from_ll(renderer, window.context()) });
        let default_pixel_format = window.window_pixel_format();
        Canvas::<Window> {
            context,
            target: window,
            default_pixel_format,
            renderer_name: unsafe {
                CStr::from_ptr(sys::render::SDL_GetRendererName(renderer))
                    .to_string_lossy()
                    .into_owned()
            },
        }
    }

    /// Returns a `TextureCreator` that can create Textures to be drawn on this `Canvas`
    ///
    /// This `TextureCreator` will share a reference to the renderer and target context.
    ///
    /// The target (i.e., `Window`) will not be destroyed and the SDL_Renderer will not be
    /// destroyed if the `TextureCreator` is still in scope.
    pub fn texture_creator(&self) -> TextureCreator<WindowContext> {
        TextureCreator {
            context: self.context.clone(),
            default_pixel_format: self.default_pixel_format(),
        }
    }
}

impl<T: RenderTarget> Canvas<T> {
    /// Temporarily sets the target of `Canvas` to a `Texture`. This effectively allows rendering
    /// to a `Texture` in any way you want: you can make a `Texture` a combination of other
    /// `Texture`s, be a complex geometry form with the `gfx` module, ... You can draw pixel by
    /// pixel in it if you want, so you can do basically anything with that `Texture`.
    ///
    /// If you want to set the content of multiple `Texture` at once the most efficient way
    /// possible, *don't* make a loop and call this function every time and use
    /// `with_multiple_texture_canvas` instead. Using `with_texture_canvas` is actually
    /// inefficient because the target is reset to the source (the `Window` or the `Surface`)
    /// at the end of this function, but using it in a loop would make this reset useless.
    /// Plus, the check that render_target is actually supported on that `Canvas` is also
    /// done every time, leading to useless checks.
    ///
    /// # Notes
    ///
    /// Note that the `Canvas` in the closure is exactly the same as the one you call this
    /// function with, meaning that you can call every function of your original `Canvas`.
    ///
    /// That means you can also call `with_texture_canvas` and `with_multiple_texture_canvas` from
    /// the inside of the closure. Even though this is useless and inefficient, this is totally
    /// safe to do and allowed.
    ///
    /// Since the render target is now a Texture, some calls of Canvas might return another result
    /// than if the target was to be the original source. For instance `output_size` will return
    /// this size of the current `Texture` in the closure, but the size of the `Window` or
    /// `Surface` outside of the closure.
    ///
    /// You do not need to call `present` after drawing in the Canvas in the closure, the changes
    /// are applied directly to the `Texture` instead of a hidden buffer.
    ///
    /// # Errors
    ///
    /// * returns `TargetRenderError::NotSupported` if the renderer does not support the use of
    /// render targets
    /// * returns `TargetRenderError::SdlError` if SDL2 returned with an error code.
    ///
    /// The texture *must* be created with the texture access:
    /// `sdl3::render::TextureAccess::Target`.
    /// Using a texture which was not created with the texture access `Target` is undefined
    /// behavior.
    ///
    /// # Examples
    ///
    /// The example below changes a newly created `Texture` to be a 150-by-150 black texture with a
    /// 50-by-50 red square in the middle.
    ///
    /// ```rust,no_run
    /// # use sdl3::render::{Canvas, Texture};
    /// # use sdl3::video::Window;
    /// # use sdl3::pixels::Color;
    /// # use sdl3::rect::Rect;
    /// # let mut canvas : Canvas<Window> = unimplemented!();
    /// let texture_creator = canvas.texture_creator();
    /// let mut texture = texture_creator
    ///     .create_texture_target(texture_creator.default_pixel_format(), 150, 150)
    ///     .unwrap();
    /// let result = canvas.with_texture_canvas(&mut texture, |texture_canvas| {
    ///     texture_canvas.set_draw_color(Color::RGBA(0, 0, 0, 255));
    ///     texture_canvas.clear();
    ///     texture_canvas.set_draw_color(Color::RGBA(255, 0, 0, 255));
    ///     texture_canvas.fill_rect(Rect::new(50, 50, 50, 50)).unwrap();
    /// }).unwrap();
    /// ```
    ///
    pub fn with_texture_canvas<F>(
        &mut self,
        texture: &mut Texture,
        f: F,
    ) -> Result<(), TargetRenderError>
    where
        for<'r> F: FnOnce(&'r mut Canvas<T>),
    {
        let target = unsafe { self.get_raw_target() };
        unsafe { self.set_raw_target(texture.raw) }.map_err(|e| TargetRenderError::SdlError(e))?;
        f(self);
        unsafe { self.set_raw_target(target) }.map_err(|e| TargetRenderError::SdlError(e))?;
        Ok(())
    }

    /// Same as `with_texture_canvas`, but allows to change multiple `Texture`s at once with the
    /// least amount of overhead. It means that between every iteration the Target is not reset to
    /// the source, and that the fact that the Canvas supports render target isn't checked every
    /// iteration either; the check is actually only done once, at the beginning, avoiding useless
    /// checks.
    ///
    /// The closure is run once for every `Texture` sent as parameter.
    ///
    /// The main changes from `with_texture_canvas` is that is takes an `Iterator` of `(&mut
    /// Texture, U)`, where U is a type defined by the user. The closure takes a `&mut Canvas`, and
    /// `&U` as arguments instead of a simple `&mut Canvas`. This user-defined type allows you to
    /// keep track of what to do with the Canvas you have received in the closure.
    ///
    /// You will usually want to keep track of the number, a property, or anything that will allow
    /// you to uniquely track this `Texture`, but it can also be an empty struct or `()` as well!
    ///
    /// # Examples
    ///
    /// Let's create two textures, one which will be yellow, and the other will be white
    ///
    /// ```rust,no_run
    /// # use sdl3::pixels::Color;
    /// # use sdl3::rect::Rect;
    /// # use sdl3::video::Window;
    /// # use sdl3::render::{Canvas, Texture};
    /// # let mut canvas : Canvas<Window> = unimplemented!();
    /// let texture_creator = canvas.texture_creator();
    /// enum TextureColor {
    ///     Yellow,
    ///     White,
    /// };
    ///
    /// let mut square_texture1 : Texture =
    ///     texture_creator.create_texture_target(None, 100, 100).unwrap();
    /// let mut square_texture2 : Texture =
    ///     texture_creator.create_texture_target(None, 100, 100).unwrap();
    /// let textures : Vec<(&mut Texture, TextureColor)> = vec![
    ///     (&mut square_texture1, TextureColor::Yellow),
    ///     (&mut square_texture2, TextureColor::White)
    /// ];
    /// let result : Result<(), _> =
    ///     canvas.with_multiple_texture_canvas(textures.iter(), |texture_canvas, user_context| {
    ///     match *user_context {
    ///         TextureColor::White => {
    ///             texture_canvas.set_draw_color(Color::RGB(255, 255, 255));
    ///         },
    ///         TextureColor::Yellow => {
    ///             texture_canvas.set_draw_color(Color::RGB(255, 255, 0));
    ///         }
    ///     };
    ///     texture_canvas.clear();
    /// });
    /// // square_texture1 is now Yellow and square_texture2 is now White!
    /// ```
    ///
    ///
    #[cfg(not(feature = "unsafe_textures"))]
    pub fn with_multiple_texture_canvas<'t: 'a, 'a: 's, 's, I, F, U: 's>(
        &mut self,
        textures: I,
        mut f: F,
    ) -> Result<(), TargetRenderError>
    where
        for<'r> F: FnMut(&'r mut Canvas<T>, &U),
        I: Iterator<Item = &'s (&'a mut Texture<'t>, U)>,
    {
        let target = unsafe { self.get_raw_target() };
        for (texture, user_context) in textures {
            unsafe { self.set_raw_target(texture.raw) }
                .map_err(|e| TargetRenderError::SdlError(e))?;
            f(self, user_context);
        }
        // reset the target to its source
        unsafe { self.set_raw_target(target) }.map_err(|e| TargetRenderError::SdlError(e))?;
        Ok(())
    }

    #[cfg(feature = "unsafe_textures")]
    pub fn with_multiple_texture_canvas<'a: 's, 's, I, F, U: 's>(
        &mut self,
        textures: I,
        mut f: F,
    ) -> Result<(), TargetRenderError>
    where
        for<'r> F: FnMut(&'r mut Canvas<T>, &U),
        I: Iterator<Item = &'s (&'a mut Texture, U)>,
    {
        for &(ref texture, ref user_context) in textures {
            unsafe { self.set_raw_target(texture.raw) }
                .map_err(|e| TargetRenderError::SdlError(e))?;
            f(self, &user_context);
        }
        // reset the target to its source
        unsafe { self.set_raw_target(ptr::null_mut()) }
            .map_err(|e| TargetRenderError::SdlError(e))?;
        Ok(())
    }
}

/// Creates Textures that cannot outlive the creator
///
/// The `TextureCreator` does not hold a lifetime to its Canvas by design choice.
///
/// If a `Canvas` is dropped before its `TextureCreator`, it is still safe to use.
///
/// It is, however, useless.
///
/// Any `Texture` created here can only be drawn onto the original `Canvas`. A `Texture` used in a
/// `Canvas` must come from a `TextureCreator` coming from that same `Canvas`. Using a `Texture` to
/// render to a `Canvas` not being the parent of the `Texture`'s `TextureCreator` is undefined
/// behavior.
pub struct TextureCreator<T> {
    context: Rc<RendererContext<T>>,
    default_pixel_format: PixelFormat,
}

/// Create a new renderer for a window.
#[doc(alias = "SDL_CreateRenderer")]
pub fn create_renderer(
    window: Window,
    renderer_name: Option<&CStr>,
) -> Result<WindowCanvas, IntegerOrSdlError> {
    use crate::common::IntegerOrSdlError::*;
    let raw = unsafe {
        sys::render::SDL_CreateRenderer(
            window.raw(),
            if let Some(renderer_name) = renderer_name {
                renderer_name.as_ptr()
            } else {
                std::ptr::null()
            },
        )
    };

    if raw.is_null() {
        Err(SdlError(get_error()))
    } else {
        Ok(Canvas::from_window_and_renderer(window, raw))
    }
}

#[derive(Debug, Clone)]
pub enum TextureValueError {
    WidthOverflows(u32),
    HeightOverflows(u32),
    WidthMustBeMultipleOfTwoForFormat(u32, PixelFormat),
    SdlError(Error),
}

impl fmt::Display for TextureValueError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::TextureValueError::*;

        match *self {
            WidthOverflows(value) => write!(f, "Integer width overflows ({})", value),
            HeightOverflows(value) => write!(f, "Integer height overflows ({})", value),
            WidthMustBeMultipleOfTwoForFormat(value, format) => {
                write!(
                    f,
                    "Texture width must be multiple of two for pixel format '{:?}' ({})",
                    format, value
                )
            }
            SdlError(ref e) => write!(f, "SDL error: {}", e),
        }
    }
}

impl error::Error for TextureValueError {
    fn description(&self) -> &str {
        use self::TextureValueError::*;

        match *self {
            WidthOverflows(_) => "texture width overflow",
            HeightOverflows(_) => "texture height overflow",
            WidthMustBeMultipleOfTwoForFormat(..) => "texture width must be multiple of two",
            SdlError(ref e) => &e.0,
        }
    }
}

#[doc(alias = "SDL_CreateTexture")]
fn ll_create_texture(
    context: *mut sys::render::SDL_Renderer,
    pixel_format: PixelFormat,
    access: TextureAccess,
    width: u32,
    height: u32,
) -> Result<*mut sys::render::SDL_Texture, TextureValueError> {
    use self::TextureValueError::*;
    let w = match validate_int(width, "width") {
        Ok(w) => w,
        Err(_) => return Err(WidthOverflows(width)),
    };
    let h = match validate_int(height, "height") {
        Ok(h) => h,
        Err(_) => return Err(HeightOverflows(height)),
    };

    // If the pixel format is YUV 4:2:0 and planar, the width and height must
    // be multiples-of-two. See issue #334 for details.
    unsafe {
        match pixel_format.raw() {
            sys::pixels::SDL_PIXELFORMAT_YV12 | sys::pixels::SDL_PIXELFORMAT_IYUV => {
                if w % 2 != 0 || h % 2 != 0 {
                    return Err(WidthMustBeMultipleOfTwoForFormat(width, pixel_format));
                }
            }
            _ => (),
        };
    }

    Ok(
        unsafe {
            sys::render::SDL_CreateTexture(context, pixel_format.into(), access.into(), w, h)
        },
    )
}

#[repr(i32)]
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum ScaleMode {
    /// nearest pixel sampling.
    Nearest = sdl3_sys::everything::SDL_ScaleMode::NEAREST.0,
    /// linear filtering. this is the default
    Linear = sdl3_sys::everything::SDL_ScaleMode::LINEAR.0,
}

impl Into<sdl3_sys::everything::SDL_ScaleMode> for ScaleMode {
    fn into(self) -> sdl3_sys::everything::SDL_ScaleMode {
        match self {
            ScaleMode::Nearest => sdl3_sys::everything::SDL_ScaleMode::NEAREST,
            ScaleMode::Linear => sdl3_sys::everything::SDL_ScaleMode::LINEAR,
        }
    }
}

impl TryFrom<sdl3_sys::everything::SDL_ScaleMode> for ScaleMode {
    type Error = ();

    fn try_from(n: sdl3_sys::everything::SDL_ScaleMode) -> Result<Self, Self::Error> {
        Ok(match n {
            sdl3_sys::everything::SDL_ScaleMode::NEAREST => Self::Nearest,
            sdl3_sys::everything::SDL_ScaleMode::LINEAR => Self::Linear,
            _ => return Err(()),
        })
    }
}

/// Texture-creating methods for the renderer
impl<T> TextureCreator<T> {
    // this can prevent introducing UB until
    // https://github.com/rust-lang/rust-clippy/issues/5953 is fixed
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub fn raw(&self) -> *mut sys::render::SDL_Renderer {
        self.context.raw()
    }

    pub fn default_pixel_format(&self) -> PixelFormat {
        self.default_pixel_format
    }

    /// Creates a texture for a rendering context.
    ///
    /// If format is `None`, the format will be the one the parent Window or Surface uses.
    ///
    /// If format is `Some(pixel_format)`, the default will be overridden, and the texture will be
    /// created with the specified format if possible. If the PixelFormat is not supported, this
    /// will return an error.
    ///
    /// You should prefer the default format if possible to have performance gains and to avoid
    /// unsupported Pixel Formats that can cause errors. However, be careful with the default
    /// `PixelFormat` if you want to create transparent textures.
    pub fn create_texture<F>(
        &self,
        format: F,
        access: TextureAccess,
        width: u32,
        height: u32,
    ) -> Result<Texture, TextureValueError>
    where
        F: Into<Option<PixelFormat>>,
    {
        use self::TextureValueError::*;
        let format: PixelFormat = format.into().unwrap_or(self.default_pixel_format);
        let result = ll_create_texture(self.context.raw(), format, access, width, height)?;
        if result.is_null() {
            Err(SdlError(get_error()))
        } else {
            unsafe { Ok(self.raw_create_texture(result)) }
        }
    }

    #[inline]
    /// Shorthand for `create_texture(format, TextureAccess::Static, width, height)`
    pub fn create_texture_static<F>(
        &self,
        format: F,
        width: u32,
        height: u32,
    ) -> Result<Texture, TextureValueError>
    where
        F: Into<Option<PixelFormat>>,
    {
        self.create_texture(format, TextureAccess::Static, width, height)
    }

    #[inline]
    /// Shorthand for `create_texture(format, TextureAccess::Streaming, width, height)`
    pub fn create_texture_streaming<F>(
        &self,
        format: F,
        width: u32,
        height: u32,
    ) -> Result<Texture, TextureValueError>
    where
        F: Into<Option<PixelFormat>>,
    {
        self.create_texture(format, TextureAccess::Streaming, width, height)
    }

    #[inline]
    /// Shorthand for `create_texture(format, TextureAccess::Target, width, height)`
    pub fn create_texture_target<F>(
        &self,
        format: F,
        width: u32,
        height: u32,
    ) -> Result<Texture, TextureValueError>
    where
        F: Into<Option<PixelFormat>>,
    {
        self.create_texture(format, TextureAccess::Target, width, height)
    }

    /// Creates a texture from an existing surface.
    ///
    /// # Remarks
    ///
    /// The access hint for the created texture is [`TextureAccess::Static`].
    ///
    /// ```no_run
    /// use sdl3::pixels::PixelFormat;
    /// use sdl3::surface::Surface;
    /// use sdl3::render::{Canvas, Texture};
    /// use sdl3::video::Window;
    ///
    /// // We init systems.
    /// let sdl_context = sdl3::init().expect("failed to init SDL");
    /// let video_subsystem = sdl_context.video().expect("failed to get video context");
    ///
    /// // We create a window.
    /// let window = video_subsystem.window("sdl3 demo", 800, 600)
    ///     .build()
    ///     .expect("failed to build window");
    ///
    /// // We get the canvas from which we can get the `TextureCreator`.
    /// let mut canvas: Canvas<Window> = window.into_canvas();
    /// let texture_creator = canvas.texture_creator();
    ///
    /// let surface = Surface::new(512, 512, PixelFormat::RGB24).unwrap();
    /// let texture = texture_creator.create_texture_from_surface(surface).unwrap();
    /// ```
    #[doc(alias = "SDL_CreateTextureFromSurface")]
    pub fn create_texture_from_surface<S: AsRef<SurfaceRef>>(
        &self,
        surface: S,
    ) -> Result<Texture, TextureValueError> {
        use self::TextureValueError::*;
        let result = unsafe {
            sys::render::SDL_CreateTextureFromSurface(self.context.raw, surface.as_ref().raw())
        };
        if result.is_null() {
            Err(SdlError(get_error()))
        } else {
            unsafe { Ok(self.raw_create_texture(result)) }
        }
    }

    /// Create a texture from its raw `SDL_Texture`.
    #[cfg(not(feature = "unsafe_textures"))]
    #[inline]
    pub const unsafe fn raw_create_texture(&self, raw: *mut sys::render::SDL_Texture) -> Texture {
        Texture {
            raw,
            _marker: PhantomData,
        }
    }

    /// Create a texture from its raw `SDL_Texture`. Should be used with care.
    #[cfg(feature = "unsafe_textures")]
    pub const unsafe fn raw_create_texture(&self, raw: *mut sys::render::SDL_Texture) -> Texture {
        Texture { raw }
    }
}

/// Drawing methods
impl<T: RenderTarget> Canvas<T> {
    // this can prevent introducing UB until
    // https://github.com/rust-lang/rust-clippy/issues/5953 is fixed
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub fn raw(&self) -> *mut sys::render::SDL_Renderer {
        self.context.raw()
    }

    /// Sets the color used for drawing operations (Rect, Line and Clear).
    #[doc(alias = "SDL_SetRenderDrawColor")]
    pub fn set_draw_color<C: Into<pixels::Color>>(&mut self, color: C) {
        let (r, g, b, a) = color.into().rgba();
        let ret = unsafe { sys::render::SDL_SetRenderDrawColor(self.raw, r, g, b, a) };
        // Should only fail on an invalid renderer
        if !ret {
            panic!("{}", get_error())
        }
    }

    /// Gets the color used for drawing operations (Rect, Line and Clear).
    #[doc(alias = "SDL_GetRenderDrawColor")]
    pub fn draw_color(&self) -> pixels::Color {
        let (mut r, mut g, mut b, mut a) = (0, 0, 0, 0);
        let ret = unsafe {
            sys::render::SDL_GetRenderDrawColor(self.context.raw, &mut r, &mut g, &mut b, &mut a)
        };
        // Should only fail on an invalid renderer
        if !ret {
            panic!("{}", get_error())
        } else {
            pixels::Color::RGBA(r, g, b, a)
        }
    }

    /// Sets the blend mode used for drawing operations (Fill and Line).
    #[doc(alias = "SDL_SetRenderDrawBlendMode")]
    pub fn set_blend_mode(&mut self, blend: BlendMode) {
        let ret =
            unsafe { sys::render::SDL_SetRenderDrawBlendMode(self.context.raw, blend as u32) };
        // Should only fail on an invalid renderer
        if !ret {
            panic!("{}", get_error())
        }
    }

    /// Gets the blend mode used for drawing operations.
    #[doc(alias = "SDL_GetRenderDrawBlendMode")]
    pub fn blend_mode(&self) -> BlendMode {
        let mut blend: MaybeUninit<SDL_BlendMode> = mem::MaybeUninit::uninit();
        let ret = unsafe {
            sys::render::SDL_GetRenderDrawBlendMode(self.context.raw, blend.as_mut_ptr())
        };
        // Should only fail on an invalid renderer
        if !ret {
            panic!("{}", get_error())
        } else {
            let blend = unsafe { blend.assume_init() };
            BlendMode::try_from(blend).unwrap()
        }
    }

    /// Clears the current rendering target with the drawing color.
    #[doc(alias = "SDL_RenderClear")]
    pub fn clear(&mut self) {
        let ret = unsafe { sys::render::SDL_RenderClear(self.context.raw) };
        if !ret {
            panic!("Could not clear: {}", get_error())
        }
    }

    /// Updates the screen with any rendering performed since the previous call.
    ///
    /// SDL's rendering functions operate on a backbuffer; that is, calling a
    /// rendering function such as `draw_line()` does not directly put a line on
    /// the screen, but rather updates the backbuffer.
    /// As such, you compose your entire scene and present the composed
    /// backbuffer to the screen as a complete picture.
    ///
    /// Returns `true` on success, or `false` on error. Call `get_error()` for more information.
    #[doc(alias = "SDL_RenderPresent")]
    pub fn present(&mut self) -> bool {
        unsafe { sys::render::SDL_RenderPresent(self.context.raw) }
    }

    /// Gets the output size of a rendering context.
    #[doc(alias = "SDL_GetCurrentRenderOutputSize")]
    pub fn output_size(&self) -> Result<(u32, u32), Error> {
        let mut width = 0;
        let mut height = 0;

        let result = unsafe {
            sys::render::SDL_GetCurrentRenderOutputSize(self.context.raw, &mut width, &mut height)
        };

        if result {
            Ok((width as u32, height as u32))
        } else {
            Err(get_error())
        }
    }

    /// Sets a device independent resolution for rendering.
    #[doc(alias = "SDL_SetRenderLogicalPresentation")]
    pub fn set_logical_size(
        &mut self,
        width: u32,
        height: u32,
        mode: sys::render::SDL_RendererLogicalPresentation,
    ) -> Result<(), IntegerOrSdlError> {
        use crate::common::IntegerOrSdlError::*;
        let width = validate_int(width, "width")?;
        let height = validate_int(height, "height")?;
        let result = unsafe {
            sys::render::SDL_SetRenderLogicalPresentation(self.context.raw, width, height, mode)
        };
        match result {
            true => Ok(()),
            false => Err(SdlError(get_error())),
        }
    }

    /// Gets device independent resolution for rendering.
    #[doc(alias = "SDL_GetRenderLogicalPresentation")]
    pub fn logical_size(&self) -> (u32, u32, sys::render::SDL_RendererLogicalPresentation) {
        let mut width = 0;
        let mut height = 0;
        let mut mode: sys::render::SDL_RendererLogicalPresentation =
            sys::render::SDL_LOGICAL_PRESENTATION_DISABLED;

        unsafe {
            sys::render::SDL_GetRenderLogicalPresentation(
                self.context.raw,
                &mut width,
                &mut height,
                &mut mode,
            )
        };

        (width as u32, height as u32, mode)
    }

    /// Sets the drawing area for rendering on the current target.
    #[doc(alias = "SDL_SetRenderViewport")]
    pub fn set_viewport<R: Into<Option<Rect>>>(&mut self, rect: R) {
        let rect = rect.into();
        // as_ref is important because we need rect to live until the end of the FFI call, but map_or consumes an Option<T>
        let ptr = rect.as_ref().map_or(ptr::null(), |rect| rect.raw());
        let ret = unsafe { sys::render::SDL_SetRenderViewport(self.context.raw, ptr) };
        if !ret {
            panic!("Could not set viewport: {}", get_error())
        }
    }

    /// Gets the drawing area for the current target.
    #[doc(alias = "SDL_GetRenderViewport")]
    pub fn viewport(&self) -> Rect {
        let mut rect = mem::MaybeUninit::uninit();
        unsafe { sys::render::SDL_GetRenderViewport(self.context.raw, rect.as_mut_ptr()) };
        let rect = unsafe { rect.assume_init() };
        Rect::from_ll(rect)
    }

    /// Sets the clip rectangle for rendering on the specified target.
    #[doc(alias = "SDL_SetRenderClipRect")]
    pub fn set_clip_rect<R>(&mut self, arg: R)
    where
        R: Into<ClippingRect>,
    {
        let arg: ClippingRect = arg.into();
        let ret = match arg {
            ClippingRect::Some(r) => unsafe {
                sdl3_sys::everything::SDL_SetRenderClipRect(self.context.raw, r.raw())
            },
            ClippingRect::Zero => {
                let r = sdl3_sys::everything::SDL_Rect {
                    x: 0,
                    y: 0,
                    w: 0,
                    h: 0,
                };
                let r: *const sdl3_sys::everything::SDL_Rect = &r;
                unsafe { sdl3_sys::everything::SDL_SetRenderClipRect(self.context.raw, r) }
            }
            ClippingRect::None => unsafe {
                sdl3_sys::everything::SDL_SetRenderClipRect(self.context.raw, ptr::null())
            },
        };
        if !ret {
            panic!("Could not set clip rect: {}", get_error())
        }
    }

    /// Gets the clip rectangle for the current target.
    #[doc(alias = "SDL_GetRenderClipRect")]
    pub fn clip_rect(&self) -> ClippingRect {
        let clip_enabled = unsafe { sdl3_sys::everything::SDL_RenderClipEnabled(self.context.raw) };

        if !clip_enabled {
            return ClippingRect::None;
        }

        let mut raw = mem::MaybeUninit::uninit();
        unsafe { sdl3_sys::everything::SDL_GetRenderClipRect(self.context.raw, raw.as_mut_ptr()) };
        let raw = unsafe { raw.assume_init() };
        if raw.w == 0 || raw.h == 0 {
            ClippingRect::Zero
        } else {
            ClippingRect::Some(Rect::from_ll(raw))
        }
    }

    /// Sets the drawing scale for rendering on the current target.
    #[doc(alias = "SDL_SetRenderScale")]
    pub fn set_scale(&mut self, scale_x: f32, scale_y: f32) -> Result<(), Error> {
        let ret = unsafe { sys::render::SDL_SetRenderScale(self.context.raw, scale_x, scale_y) };
        // Should only fail on an invalid renderer
        if !ret {
            Err(get_error())
        } else {
            Ok(())
        }
    }

    /// Gets the drawing scale for the current target.
    #[doc(alias = "SDL_GetRenderScale")]
    pub fn scale(&self) -> (f32, f32) {
        let mut scale_x = 0.0;
        let mut scale_y = 0.0;
        unsafe { sys::render::SDL_GetRenderScale(self.context.raw, &mut scale_x, &mut scale_y) };
        (scale_x, scale_y)
    }

    /// Draws a point on the current rendering target.
    /// Errors if drawing fails for any reason (e.g. driver failure)
    #[doc(alias = "SDL_RenderPoint")]
    pub fn draw_point<P: Into<FPoint>>(&mut self, point: P) -> Result<(), Error> {
        let point = point.into();
        let result = unsafe { sys::render::SDL_RenderPoint(self.context.raw, point.x, point.y) };
        if !result {
            Err(get_error())
        } else {
            Ok(())
        }
    }

    /// Draws multiple points on the current rendering target.
    /// Errors if drawing fails for any reason (e.g. driver failure)
    #[doc(alias = "SDL_RenderPoints")]
    pub fn draw_points<'a, P: Into<&'a [FPoint]>>(&mut self, points: P) -> Result<(), Error> {
        let points = points.into();
        let result = unsafe {
            sys::render::SDL_RenderPoints(
                self.context.raw,
                points.as_ptr() as *const sys::rect::SDL_FPoint,
                points.len() as c_int,
            )
        };
        if !result {
            Err(get_error())
        } else {
            Ok(())
        }
    }

    /// Draws a line on the current rendering target.
    /// Errors if drawing fails for any reason (e.g. driver failure)
    #[doc(alias = "SDL_RenderLine")]
    pub fn draw_line<P1: Into<FPoint>, P2: Into<FPoint>>(
        &mut self,
        start: P1,
        end: P2,
    ) -> Result<(), Error> {
        let start = start.into();
        let end = end.into();
        let result = unsafe {
            sys::render::SDL_RenderLine(self.context.raw, start.x, start.y, end.x, end.y)
        };
        if !result {
            Err(get_error())
        } else {
            Ok(())
        }
    }

    /// Draws a series of connected lines on the current rendering target.
    /// Errors if drawing fails for any reason (e.g. driver failure)
    #[doc(alias = "SDL_RenderLines")]
    pub fn draw_lines<'a, P: Into<&'a [FPoint]>>(&mut self, points: P) -> Result<(), Error> {
        let points = points.into();
        let result = unsafe {
            sys::render::SDL_RenderLines(
                self.context.raw,
                points
                    .iter()
                    .map(|p| p.to_ll())
                    .collect::<Vec<_>>()
                    .as_ptr(),
                points.len() as c_int,
            )
        };
        if !result {
            Err(get_error())
        } else {
            Ok(())
        }
    }

    /// Draws a rectangle on the current rendering target.
    /// Errors if drawing fails for any reason (e.g. driver failure)
    #[doc(alias = "SDL_RenderRect")]
    pub fn draw_rect(&mut self, rect: FRect) -> Result<(), Error> {
        let rect = rect.to_ll();

        let result = unsafe { sys::render::SDL_RenderRect(self.context.raw, &rect) };
        if !result {
            Err(get_error())
        } else {
            Ok(())
        }
    }

    /// Draws some number of rectangles on the current rendering target.
    /// Errors if drawing fails for any reason (e.g. driver failure)
    #[doc(alias = "SDL_RenderRects")]
    pub fn draw_rects(&mut self, rects: &[FRect]) -> Result<(), Error> {
        let result = unsafe {
            sys::render::SDL_RenderRects(
                self.context.raw,
                rects.iter().map(|r| r.to_ll()).collect::<Vec<_>>().as_ptr(),
                rects.len() as c_int,
            )
        };
        if !result {
            Err(get_error())
        } else {
            Ok(())
        }
    }

    /// Fills a rectangle on the current rendering target with the drawing
    /// color.
    /// Passing None will fill the entire rendering target.
    /// Errors if drawing fails for any reason (e.g. driver failure)
    #[doc(alias = "SDL_RenderFillRect")]
    pub fn fill_rect<R: Into<Option<FRect>>>(&mut self, rect: R) -> Result<(), Error> {
        let rect_ll = rect.into().map(|r| r.to_ll());
        let result = unsafe {
            sys::render::SDL_RenderFillRect(
                self.context.raw,
                rect_ll.as_ref().map_or(ptr::null(), |r| r),
            )
        };
        if !result {
            Err(get_error())
        } else {
            Ok(())
        }
    }

    /// Fills some number of rectangles on the current rendering target with
    /// the drawing color.
    /// Errors if drawing fails for any reason (e.g. driver failure)
    #[doc(alias = "SDL_RenderFillRects")]
    pub fn fill_rects(&mut self, rects: &[FRect]) -> Result<(), Error> {
        let result = unsafe {
            sys::render::SDL_RenderFillRects(
                self.context.raw,
                rects.iter().map(|r| r.to_ll()).collect::<Vec<_>>().as_ptr(),
                rects.len() as c_int,
            )
        };
        if !result {
            Err(get_error())
        } else {
            Ok(())
        }
    }

    /// Copies a portion of the texture to the current rendering target.
    ///
    /// * If `src` is `None`, the entire texture is copied.
    /// * If `dst` is `None`, the texture will be stretched to fill the given
    ///   rectangle.
    ///
    /// Errors if drawing fails for any reason (e.g. driver failure),
    /// or if the provided texture does not belong to the renderer.
    #[doc(alias = "SDL_RenderTexture")]
    pub fn copy<R1, R2>(&mut self, texture: &Texture, src: R1, dst: R2) -> Result<(), Error>
    where
        R1: Into<Option<FRect>>,
        R2: Into<Option<FRect>>,
    {
        let src = src.into().map(|rect| rect.to_ll());
        let dst = dst.into().map(|rect| rect.to_ll());

        let ret = unsafe {
            sys::render::SDL_RenderTexture(
                self.context.raw,
                texture.raw,
                match src {
                    Some(ref rect) => rect,
                    None => ptr::null(),
                },
                match dst {
                    Some(ref rect) => rect,
                    None => ptr::null(),
                },
            )
        };

        if !ret {
            Err(get_error())
        } else {
            Ok(())
        }
    }

    /// Copies a portion of the texture to the current rendering target,
    /// optionally rotating it by angle around the given center and also
    /// flipping it top-bottom and/or left-right.
    ///
    /// * If `src` is `None`, the entire texture is copied.
    /// * If `dst` is `None`, the texture will be stretched to fill the given
    ///   rectangle.
    /// * If `center` is `None`, rotation will be done around the center point
    ///   of `dst`, or `src` if `dst` is None.
    ///
    /// Errors if drawing fails for any reason (e.g. driver failure),
    /// if the provided texture does not belong to the renderer,
    /// or if the driver does not support RenderCopyEx.
    #[doc(alias = "SDL_RenderTextureRotated")]
    pub fn copy_ex<R1, R2, P>(
        &mut self,
        texture: &Texture,
        src: R1,
        dst: R2,
        angle: f64,
        center: P,
        flip_horizontal: bool,
        flip_vertical: bool,
    ) -> Result<(), Error>
    where
        R1: Into<Option<FRect>>,
        R2: Into<Option<FRect>>,
        P: Into<Option<FPoint>>,
    {
        let flip = unsafe {
            match (flip_horizontal, flip_vertical) {
                (false, false) => SDL_FLIP_NONE,
                (true, false) => SDL_FLIP_HORIZONTAL,
                (false, true) => SDL_FLIP_VERTICAL,
                (true, true) => transmute::<u32, sys::surface::SDL_FlipMode>(
                    transmute::<sys::surface::SDL_FlipMode, u32>(SDL_FLIP_HORIZONTAL)
                        | transmute::<sys::surface::SDL_FlipMode, u32>(SDL_FLIP_VERTICAL),
                ),
            }
        };

        let src = src.into().map(|rect| rect.to_ll());
        let dst = dst.into().map(|rect| rect.to_ll());
        let center = center.into().map(|point| point.to_ll());

        let ret = unsafe {
            sys::render::SDL_RenderTextureRotated(
                self.context.raw,
                texture.raw,
                match src {
                    Some(ref rect) => rect,
                    None => ptr::null(),
                },
                match dst {
                    Some(ref rect) => rect,
                    None => ptr::null(),
                },
                angle as c_double,
                match center {
                    Some(ref point) => point,
                    None => ptr::null(),
                },
                flip,
            )
        };

        if !ret {
            Err(get_error())
        } else {
            Ok(())
        }
    }

    /// Reads pixels from the current rendering target.
    /// # Remarks
    /// WARNING: This is a very slow operation, and should not be used frequently.
    #[doc(alias = "SDL_RenderReadPixels")]
    pub fn read_pixels<R: Into<Option<Rect>>>(
        &self,
        rect: R,
        // format: pixels::PixelFormat,
    ) -> Result<Surface, Error> {
        unsafe {
            let rect = rect.into();
            let (actual_rect, _w, _h) = match rect {
                Some(ref rect) => (rect.raw(), rect.width() as usize, rect.height() as usize),
                None => {
                    let (w, h) = self.output_size()?;
                    (ptr::null(), w as usize, h as usize)
                }
            };

            let surface_ptr = sys::render::SDL_RenderReadPixels(self.context.raw, actual_rect);
            if surface_ptr.is_null() {
                return Err(get_error());
            }

            let surface = Surface::from_ll(surface_ptr);
            Ok(surface)
        }
    }

    /// Creates a texture for a rendering context.
    ///
    /// If format is `None`, the format will be the one the parent Window or Surface uses.
    ///
    /// If format is `Some(pixel_format)`
    /// created with the specified format if possible. If the PixelFormat is not supported, this
    /// will return an error.
    ///
    /// You should prefer the default format if possible to have performance gains and to avoid
    /// unsupported Pixel Formats that can cause errors. However, be careful with the default
    /// `PixelFormat` if you want to create transparent textures.
    ///
    /// # Notes
    ///
    /// Note that this method is only accessible in Canvas with the `unsafe_textures` feature,
    /// because lifetimes otherwise prevent `Canvas` from creating and accessing `Texture`s at the
    /// same time.
    #[cfg(feature = "unsafe_textures")]
    pub fn create_texture<F>(
        &self,
        format: F,
        access: TextureAccess,
        width: u32,
        height: u32,
    ) -> Result<Texture, TextureValueError>
    where
        F: Into<Option<PixelFormat>>,
    {
        use self::TextureValueError::*;
        let format: PixelFormat = format.into().unwrap_or(self.default_pixel_format);
        let result = ll_create_texture(self.context.raw(), format, access, width, height)?;
        if result.is_null() {
            Err(SdlError(get_error()))
        } else {
            unsafe { Ok(self.raw_create_texture(result)) }
        }
    }

    /// Shorthand for `create_texture(format, TextureAccess::Static, width, height)`
    ///
    /// # Notes
    ///
    /// Note that this method is only accessible in Canvas with the `unsafe_textures` feature.
    #[cfg(feature = "unsafe_textures")]
    #[inline]
    pub fn create_texture_static<F>(
        &self,
        format: F,
        width: u32,
        height: u32,
    ) -> Result<Texture, TextureValueError>
    where
        F: Into<Option<PixelFormat>>,
    {
        self.create_texture(format, TextureAccess::Static, width, height)
    }

    /// Shorthand for `create_texture(format, TextureAccess::Streaming, width, height)`
    ///
    /// # Notes
    ///
    /// Note that this method is only accessible in Canvas with the `unsafe_textures` feature.
    #[cfg(feature = "unsafe_textures")]
    #[inline]
    pub fn create_texture_streaming<F>(
        &self,
        format: F,
        width: u32,
        height: u32,
    ) -> Result<Texture, TextureValueError>
    where
        F: Into<Option<PixelFormat>>,
    {
        self.create_texture(format, TextureAccess::Streaming, width, height)
    }

    /// Shorthand for `create_texture(format, TextureAccess::Target, width, height)`
    ///
    /// # Notes
    ///
    /// Note that this method is only accessible in Canvas with the `unsafe_textures` feature.
    #[cfg(feature = "unsafe_textures")]
    #[inline]
    pub fn create_texture_target<F>(
        &self,
        format: F,
        width: u32,
        height: u32,
    ) -> Result<Texture, TextureValueError>
    where
        F: Into<Option<PixelFormat>>,
    {
        self.create_texture(format, TextureAccess::Target, width, height)
    }

    /// Creates a texture from an existing surface.
    ///
    /// # Remarks
    ///
    /// The access hint for the created texture is `TextureAccess::Static`.
    ///
    /// # Notes
    ///
    /// Note that this method is only accessible in Canvas with the `unsafe_textures` feature.
    #[cfg(feature = "unsafe_textures")]
    #[doc(alias = "SDL_CreateTextureFromSurface")]
    pub fn create_texture_from_surface<S: AsRef<SurfaceRef>>(
        &self,
        surface: S,
    ) -> Result<Texture, TextureValueError> {
        use self::TextureValueError::*;
        let result = unsafe {
            sys::render::SDL_CreateTextureFromSurface(self.context.raw, surface.as_ref().raw())
        };
        if result.is_null() {
            Err(SdlError(get_error()))
        } else {
            unsafe { Ok(self.raw_create_texture(result)) }
        }
    }

    #[cfg(feature = "unsafe_textures")]
    /// Create a texture from its raw `SDL_Texture`. Should be used with care.
    ///
    /// # Notes
    ///
    /// Note that this method is only accessible in Canvas with the `unsafe_textures` feature.
    pub unsafe fn raw_create_texture(&self, raw: *mut sys::render::SDL_Texture) -> Texture {
        Texture { raw }
    }

    #[doc(alias = "SDL_FlushRenderer")]
    pub unsafe fn flush_renderer(&self) {
        let ret = sys::render::SDL_FlushRenderer(self.context.raw);

        if !ret {
            panic!("Error flushing renderer: {}", get_error())
        }
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct TextureQuery {
    pub format: pixels::PixelFormat,
    pub access: TextureAccess,
    pub width: u32,
    pub height: u32,
}

/// A texture for a rendering context.
///
/// Every Texture is owned by a `TextureCreator` or `Canvas` (the latter is only possible with the
/// `unsafe_textures` feature).
///
/// # Differences between with and without `unsafe_textures` feature
///
/// Without the `unsafe_textures`, a texture is owned by a `TextureCreator` and a `Texture` cannot
/// outlive its parent `TextureCreator` thanks to lifetimes. A texture is destroyed via its `Drop`
/// implementation. While this is the most "Rust"-y way of doing things currently, it is pretty
/// cumbersome to use in some cases.
///
/// That is why the feature `unsafe_textures` was brought to life: the lifetimes are gone, meaning
/// that `Texture`s *can* outlive their parents. That means that the `Texture`s are not destroyed
/// on `Drop`, but destroyed when their parents are. That means if you create 10 000 textures with
/// this feature, they will only be destroyed after you drop the `Canvas` and every
/// `TextureCreator` linked to it. While this feature is enabled, this is the safest way to free
/// the memory taken by the `Texture`s, but there is still another, unsafe way to destroy the
/// `Texture` before its `Canvas`: the method `destroy`. This method is unsafe because *you* have
/// to make sure the parent `Canvas` or `TextureCreator` is still alive while calling this method.
///
/// **Calling the `destroy` method while no parent is alive is undefined behavior**
///
/// With the `unsafe_textures` feature, a `Texture` can be safely accessed (but not destroyed) after
/// the `Canvas` is dropped, but since any access (except `destroy`) requires the original `Canvas`,
/// it is not possible to access a `Texture` while the `Canvas` is dropped.
#[cfg(feature = "unsafe_textures")]
pub struct Texture {
    raw: *mut sys::render::SDL_Texture,
}

/// A texture for a rendering context.
///
/// Every Texture is owned by a `TextureCreator`. Internally, a texture is destroyed via its `Drop`
/// implementation. A texture can only be used by the `Canvas` it was originally created from, it
/// is undefined behavior otherwise.
#[cfg(not(feature = "unsafe_textures"))]
pub struct Texture<'r> {
    raw: *mut sys::render::SDL_Texture,
    _marker: PhantomData<&'r ()>,
}

#[cfg(not(feature = "unsafe_textures"))]
impl Drop for Texture<'_> {
    #[doc(alias = "SDL_DestroyTexture")]
    fn drop(&mut self) {
        unsafe {
            sys::render::SDL_DestroyTexture(self.raw);
        }
    }
}

#[cfg(feature = "unsafe_textures")]
impl Texture {
    /// Destroy the Texture and its representation
    /// in the Renderer. This will most likely
    /// mean that the renderer engine will free video
    /// memory that was allocated for this texture.
    ///
    /// This method is unsafe because since Texture does not have
    /// a lifetime, it is legal in Rust to make this texture live
    /// longer than the Renderer. It is however illegal to destroy a SDL_Texture
    /// after its SDL_Renderer, therefore this function is unsafe because
    /// of this.
    ///
    /// Note however that you don't *have* to destroy a Texture before its Canvas,
    /// since whenever Canvas is destroyed, the SDL implementation will automatically
    /// destroy all the children Textures of that Canvas.
    ///
    /// **Calling this method while no parent is alive is undefined behavior**
    pub unsafe fn destroy(self) {
        sys::render::SDL_DestroyTexture(self.raw)
    }
}

#[derive(Debug, Clone)]
pub enum UpdateTextureError {
    PitchOverflows(usize),
    PitchMustBeMultipleOfTwoForFormat(usize, PixelFormat),
    XMustBeMultipleOfTwoForFormat(i32, PixelFormat),
    YMustBeMultipleOfTwoForFormat(i32, PixelFormat),
    WidthMustBeMultipleOfTwoForFormat(u32, PixelFormat),
    HeightMustBeMultipleOfTwoForFormat(u32, PixelFormat),
    SdlError(Error),
}

impl fmt::Display for UpdateTextureError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::UpdateTextureError::*;

        match *self {
            PitchOverflows(value) => write!(f, "Pitch overflows ({})", value),
            PitchMustBeMultipleOfTwoForFormat(value, format) => {
                write!(
                    f,
                    "Pitch must be multiple of two for pixel format '{:?}' ({})",
                    format, value
                )
            }
            XMustBeMultipleOfTwoForFormat(value, format) => {
                write!(
                    f,
                    "X must be multiple of two for pixel format '{:?}' ({})",
                    format, value
                )
            }
            YMustBeMultipleOfTwoForFormat(value, format) => {
                write!(
                    f,
                    "Y must be multiple of two for pixel format '{:?}' ({})",
                    format, value
                )
            }
            WidthMustBeMultipleOfTwoForFormat(value, format) => {
                write!(
                    f,
                    "Width must be multiple of two for pixel format '{:?}' ({})",
                    format, value
                )
            }
            HeightMustBeMultipleOfTwoForFormat(value, format) => {
                write!(
                    f,
                    "Height must be multiple of two for pixel format '{:?}' ({})",
                    format, value
                )
            }
            SdlError(ref e) => write!(f, "SDL error: {}", e),
        }
    }
}

impl error::Error for UpdateTextureError {
    fn description(&self) -> &str {
        use self::UpdateTextureError::*;

        match *self {
            PitchOverflows(_) => "pitch overflow",
            PitchMustBeMultipleOfTwoForFormat(..) => "pitch must be multiple of two",
            XMustBeMultipleOfTwoForFormat(..) => "x must be multiple of two",
            YMustBeMultipleOfTwoForFormat(..) => "y must be multiple of two",
            WidthMustBeMultipleOfTwoForFormat(..) => "width must be multiple of two",
            HeightMustBeMultipleOfTwoForFormat(..) => "height must be multiple of two",
            SdlError(ref e) => &e.0,
        }
    }
}

#[derive(Debug, Clone)]
pub enum UpdateTextureYUVError {
    PitchOverflows {
        plane: &'static str,
        value: usize,
    },
    InvalidPlaneLength {
        plane: &'static str,
        length: usize,
        pitch: usize,
        height: usize,
    },
    XMustBeMultipleOfTwoForFormat(i32),
    YMustBeMultipleOfTwoForFormat(i32),
    WidthMustBeMultipleOfTwoForFormat(u32),
    HeightMustBeMultipleOfTwoForFormat(u32),
    RectNotInsideTexture(Rect),
    SdlError(Error),
}

impl fmt::Display for UpdateTextureYUVError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::UpdateTextureYUVError::*;

        match *self {
            PitchOverflows { plane, value } => {
                write!(f, "Pitch overflows on {} plane ({})", plane, value)
            }
            InvalidPlaneLength {
                plane,
                length,
                pitch,
                height,
            } => {
                write!(
                    f,
                    "The {} plane is wrong length ({}, should be {} * {})",
                    plane, length, pitch, height
                )
            }
            XMustBeMultipleOfTwoForFormat(value) => {
                write!(f, "X must be multiple of two ({})", value)
            }
            YMustBeMultipleOfTwoForFormat(value) => {
                write!(f, "Y must be multiple of two ({})", value)
            }
            WidthMustBeMultipleOfTwoForFormat(value) => {
                write!(f, "Width must be multiple of two ({})", value)
            }
            HeightMustBeMultipleOfTwoForFormat(value) => {
                write!(f, "Height must be multiple of two ({})", value)
            }
            RectNotInsideTexture(_) => write!(f, "Rect must be inside texture"),
            SdlError(ref e) => write!(f, "SDL error: {}", e),
        }
    }
}

impl error::Error for UpdateTextureYUVError {
    fn description(&self) -> &str {
        use self::UpdateTextureYUVError::*;

        match *self {
            PitchOverflows { .. } => "pitch overflow",
            InvalidPlaneLength { .. } => "invalid plane length",
            XMustBeMultipleOfTwoForFormat(_) => "x must be multiple of two",
            YMustBeMultipleOfTwoForFormat(_) => "y must be multiple of two",
            WidthMustBeMultipleOfTwoForFormat(_) => "width must be multiple of two",
            HeightMustBeMultipleOfTwoForFormat(_) => "height must be multiple of two",
            RectNotInsideTexture(_) => "rect must be inside texture",
            SdlError(ref e) => &e.0,
        }
    }
}

struct InternalTexture {
    raw: *mut sys::render::SDL_Texture,
}

impl InternalTexture {
    #[doc(alias = "SDL_GetTextureProperties")]
    pub fn get_properties(&self) -> SDL_PropertiesID {
        unsafe { SDL_GetTextureProperties(self.raw) }
    }

    pub fn get_format(&self) -> PixelFormat {
        let format = unsafe {
            sys::properties::SDL_GetNumberProperty(
                self.get_properties(),
                sys::render::SDL_PROP_TEXTURE_FORMAT_NUMBER,
                0,
            )
        };

        PixelFormat::from(format)
    }

    pub fn get_access(&self) -> TextureAccess {
        let access = unsafe {
            sys::properties::SDL_GetNumberProperty(
                self.get_properties(),
                sys::render::SDL_PROP_TEXTURE_ACCESS_NUMBER,
                0,
            )
        };
        TextureAccess::from(access)
    }

    pub fn get_width(&self) -> u32 {
        unsafe {
            sys::properties::SDL_GetNumberProperty(
                self.get_properties(),
                sys::render::SDL_PROP_TEXTURE_WIDTH_NUMBER,
                0,
            ) as u32
        }
    }

    pub fn get_height(&self) -> u32 {
        unsafe {
            sys::properties::SDL_GetNumberProperty(
                self.get_properties(),
                sys::render::SDL_PROP_TEXTURE_HEIGHT_NUMBER,
                0,
            ) as u32
        }
    }

    #[doc(alias = "SDL_SetTextureColorMod")]
    pub fn set_color_mod(&mut self, red: u8, green: u8, blue: u8) {
        let ret = unsafe { sys::render::SDL_SetTextureColorMod(self.raw, red, green, blue) };

        if !ret {
            panic!("Error setting color mod: {}", get_error())
        }
    }

    #[doc(alias = "SDL_GetTextureColorMod")]
    pub fn color_mod(&self) -> (u8, u8, u8) {
        let (mut r, mut g, mut b) = (0, 0, 0);
        let ret = unsafe { sys::render::SDL_GetTextureColorMod(self.raw, &mut r, &mut g, &mut b) };

        // Should only fail on an invalid texture
        if !ret {
            panic!("{}", get_error())
        } else {
            (r, g, b)
        }
    }

    #[doc(alias = "SDL_SetTextureAlphaMod")]
    pub fn set_alpha_mod(&mut self, alpha: u8) {
        let ret = unsafe { sys::render::SDL_SetTextureAlphaMod(self.raw, alpha) };

        if !ret {
            panic!("Error setting alpha mod: {}", get_error())
        }
    }

    #[doc(alias = "SDL_GetTextureAlphaMod")]
    pub fn alpha_mod(&self) -> u8 {
        let mut alpha = 0;
        let ret = unsafe { sys::render::SDL_GetTextureAlphaMod(self.raw, &mut alpha) };

        // Should only fail on an invalid texture
        if !ret {
            panic!("{}", get_error())
        } else {
            alpha
        }
    }

    #[doc(alias = "SDL_SetTextureBlendMode")]
    pub fn set_blend_mode(&mut self, blend: BlendMode) {
        let ret = unsafe { sys::render::SDL_SetTextureBlendMode(self.raw, blend as u32) };

        if !ret {
            panic!("Error setting blend: {}", get_error())
        }
    }

    #[doc(alias = "SDL_GetTextureBlendMode")]
    pub fn blend_mode(&self) -> BlendMode {
        let mut blend: MaybeUninit<SDL_BlendMode> = mem::MaybeUninit::uninit();
        let ret = unsafe { sys::render::SDL_GetTextureBlendMode(self.raw, blend.as_mut_ptr()) };

        // Should only fail on an invalid texture
        if !ret {
            panic!("{}", get_error())
        } else {
            let blend = unsafe { blend.assume_init() };
            BlendMode::try_from(blend).unwrap()
        }
    }

    #[doc(alias = "SDL_SetTextureScaleMode")]
    pub fn set_scale_mode(&mut self, scale: ScaleMode) {
        let ret = unsafe { sdl3_sys::everything::SDL_SetTextureScaleMode(self.raw, scale.into()) };

        if !ret {
            panic!("Error setting scale mode: {}", get_error())
        }
    }

    #[doc(alias = "SDL_GetTextureScaleMode")]
    pub fn scale_mode(&self) -> ScaleMode {
        let mut scale: MaybeUninit<sdl3_sys::everything::SDL_ScaleMode> =
            mem::MaybeUninit::uninit();
        let ret =
            unsafe { sdl3_sys::everything::SDL_GetTextureScaleMode(self.raw, scale.as_mut_ptr()) };
        if !ret {
            panic!("{}", get_error())
        } else {
            let scale = unsafe { scale.assume_init() };
            ScaleMode::try_from(scale).unwrap()
        }
    }

    #[doc(alias = "SDL_UpdateTexture")]
    pub fn update<R>(
        &mut self,
        rect: R,
        pixel_data: &[u8],
        pitch: usize,
    ) -> Result<(), UpdateTextureError>
    where
        R: Into<Option<Rect>>,
    {
        use self::UpdateTextureError::*;
        let rect = rect.into();
        let rect_raw_ptr = match rect {
            Some(ref rect) => rect.raw(),
            None => ptr::null(),
        };

        // Check if the rectangle's position or size is odd, and if the pitch is odd.
        // This needs to be done in case the texture's pixel format is planar YUV.
        // See issue #334 for details.
        let format = self.get_format();
        unsafe {
            match format.raw() {
                sys::pixels::SDL_PIXELFORMAT_YV12 | sys::pixels::SDL_PIXELFORMAT_IYUV => {
                    if let Some(r) = rect {
                        if r.x() % 2 != 0 {
                            return Err(XMustBeMultipleOfTwoForFormat(r.x(), format));
                        } else if r.y() % 2 != 0 {
                            return Err(YMustBeMultipleOfTwoForFormat(r.y(), format));
                        } else if r.width() % 2 != 0 {
                            return Err(WidthMustBeMultipleOfTwoForFormat(r.width(), format));
                        } else if r.height() % 2 != 0 {
                            return Err(HeightMustBeMultipleOfTwoForFormat(r.height(), format));
                        }
                    };
                    if pitch % 2 != 0 {
                        return Err(PitchMustBeMultipleOfTwoForFormat(pitch, format));
                    }
                }
                _ => {}
            }
        }

        let pitch = match validate_int(pitch as u32, "pitch") {
            Ok(p) => p,
            Err(_) => return Err(PitchOverflows(pitch)),
        };

        let result = unsafe {
            sys::render::SDL_UpdateTexture(
                self.raw,
                rect_raw_ptr,
                pixel_data.as_ptr() as *const _,
                pitch,
            )
        };

        if !result {
            Err(SdlError(get_error()))
        } else {
            Ok(())
        }
    }

    #[doc(alias = "SDL_UpdateYUVTexture")]
    pub fn update_yuv<R>(
        &mut self,
        rect: R,
        y_plane: &[u8],
        y_pitch: usize,
        u_plane: &[u8],
        u_pitch: usize,
        v_plane: &[u8],
        v_pitch: usize,
    ) -> Result<(), UpdateTextureYUVError>
    where
        R: Into<Option<Rect>>,
    {
        use self::UpdateTextureYUVError::*;

        let rect = rect.into();

        let rect_raw_ptr = match rect {
            Some(ref rect) => rect.raw(),
            None => ptr::null(),
        };

        if let Some(ref r) = rect {
            if r.x() % 2 != 0 {
                return Err(XMustBeMultipleOfTwoForFormat(r.x()));
            } else if r.y() % 2 != 0 {
                return Err(YMustBeMultipleOfTwoForFormat(r.y()));
            } else if r.width() % 2 != 0 {
                return Err(WidthMustBeMultipleOfTwoForFormat(r.width()));
            } else if r.height() % 2 != 0 {
                return Err(HeightMustBeMultipleOfTwoForFormat(r.height()));
            }
        };

        // If the destination rectangle lies outside the texture boundaries,
        // SDL_UpdateYUVTexture will write outside allocated texture memory.
        let width_ = self.get_width();
        let height_ = self.get_height();
        if let Some(ref r) = rect {
            let tex_rect = Rect::new(0, 0, width_, height_);
            let inside = match r.intersection(tex_rect) {
                Some(intersection) => intersection == *r,
                None => false,
            };
            // The destination rectangle cannot lie outside the texture boundaries
            if !inside {
                return Err(RectNotInsideTexture(*r));
            }
        }

        // We need the height in order to check the array slice lengths.
        // Checking the lengths can prevent buffer overruns in SDL_UpdateYUVTexture.
        let height = match rect {
            Some(ref r) => r.height(),
            None => height_,
        } as usize;

        //let wrong_length =
        if y_plane.len() != (y_pitch * height) {
            return Err(InvalidPlaneLength {
                plane: "y",
                length: y_plane.len(),
                pitch: y_pitch,
                height,
            });
        }
        if u_plane.len() != (u_pitch * height / 2) {
            return Err(InvalidPlaneLength {
                plane: "u",
                length: u_plane.len(),
                pitch: u_pitch,
                height: height / 2,
            });
        }
        if v_plane.len() != (v_pitch * height / 2) {
            return Err(InvalidPlaneLength {
                plane: "v",
                length: v_plane.len(),
                pitch: v_pitch,
                height: height / 2,
            });
        }

        let y_pitch = match validate_int(y_pitch as u32, "y_pitch") {
            Ok(p) => p,
            Err(_) => {
                return Err(PitchOverflows {
                    plane: "y",
                    value: y_pitch,
                })
            }
        };
        let u_pitch = match validate_int(u_pitch as u32, "u_pitch") {
            Ok(p) => p,
            Err(_) => {
                return Err(PitchOverflows {
                    plane: "u",
                    value: u_pitch,
                })
            }
        };
        let v_pitch = match validate_int(v_pitch as u32, "v_pitch") {
            Ok(p) => p,
            Err(_) => {
                return Err(PitchOverflows {
                    plane: "v",
                    value: v_pitch,
                })
            }
        };

        let result = unsafe {
            sys::render::SDL_UpdateYUVTexture(
                self.raw,
                rect_raw_ptr,
                y_plane.as_ptr(),
                y_pitch,
                u_plane.as_ptr(),
                u_pitch,
                v_plane.as_ptr(),
                v_pitch,
            )
        };
        if !result {
            Err(SdlError(get_error()))
        } else {
            Ok(())
        }
    }

    #[doc(alias = "SDL_LockTexture")]
    pub fn with_lock<F, R, R2>(&mut self, rect: R2, func: F) -> Result<R, Error>
    where
        F: FnOnce(&mut [u8], usize) -> R,
        R2: Into<Option<Rect>>,
    {
        // Call to SDL to populate pixel data
        let loaded = unsafe {
            let mut pixels = ptr::null_mut();
            let mut pitch = 0;
            let height = self.get_height();
            let format = self.get_format();

            let (rect_raw_ptr, height) = match rect.into() {
                Some(ref rect) => (rect.raw(), rect.height() as usize),
                None => (ptr::null(), height as usize),
            };

            let ret = sys::render::SDL_LockTexture(self.raw, rect_raw_ptr, &mut pixels, &mut pitch);
            if ret {
                let size = format.byte_size_from_pitch_and_height(pitch as usize, height);
                Ok((
                    ::std::slice::from_raw_parts_mut(pixels as *mut u8, size),
                    pitch,
                ))
            } else {
                Err(get_error())
            }
        };

        match loaded {
            Ok((interior, pitch)) => {
                let result;
                unsafe {
                    result = func(interior, pitch as usize);
                    sys::render::SDL_UnlockTexture(self.raw);
                }
                Ok(result)
            }
            Err(e) => Err(e),
        }
    }

    // not really sure about this!
    unsafe fn get_gl_texture_id(&self) -> Sint64 {
        let props_id = unsafe { SDL_GetTextureProperties(self.raw) };
        unsafe {
            sys::properties::SDL_GetNumberProperty(
                props_id,
                sys::render::SDL_PROP_TEXTURE_OPENGL_TEXTURE_NUMBER,
                0,
            )
        }
    }

    // removed:
    // SDL_GL_BindTexture() - use SDL_GetTextureProperties() to get the OpenGL texture ID and bind the texture directly
    // SDL_GL_UnbindTexture() - use SDL_GetTextureProperties() to get the OpenGL texture ID and unbind the texture directly

    // pub unsafe fn gl_bind_texture(&mut self) -> (f32, f32) {
    //     let mut texw = 0.0;
    //     let mut texh = 0.0;
    //
    //     if sys::render::SDL_GL_BindTexture(self.raw, &mut texw, &mut texh) == 0 {
    //         (texw, texh)
    //     } else {
    //         panic!("OpenGL texture binding not supported");
    //     }
    // }
    //
    // pub unsafe fn gl_unbind_texture(&mut self) {
    //     if sys::render::SDL_GL_UnbindTexture(self.raw) != 0 {
    //         panic!("OpenGL texture unbinding not supported");
    //     }
    // }

    // #[doc(alias = "SDL_GL_BindTexture")]
    // pub fn gl_with_bind<R, F: FnOnce(f32, f32) -> R>(&mut self, f: F) -> R {
    //     unsafe {
    //         let mut texw = 0.0;
    //         let mut texh = 0.0;
    //
    //         if sys::render::SDL_GL_BindTexture(self.raw, &mut texw, &mut texh) == 0 {
    //             let return_value = f(texw, texh);
    //
    //             if sys::render::SDL_GL_UnbindTexture(self.raw) == 0 {
    //                 return_value
    //             } else {
    //                 // This should never happen...
    //                 panic!();
    //             }
    //         } else {
    //             panic!("OpenGL texture binding not supported");
    //         }
    //     }
    // }
}

#[cfg(not(feature = "unsafe_textures"))]
impl Texture<'_> {
    /// Gets the texture's internal properties.
    #[inline]
    pub fn query(&self) -> TextureQuery {
        let internal = InternalTexture { raw: self.raw };
        TextureQuery {
            format: internal.get_format(),
            access: internal.get_access(),
            width: internal.get_width(),
            height: internal.get_height(),
        }
    }

    /// Get the format of the texture.
    #[inline]
    pub fn format(&self) -> PixelFormat {
        InternalTexture { raw: self.raw }.get_format()
    }

    /// Get the access of the texture.
    #[inline]
    pub fn access(&self) -> TextureAccess {
        InternalTexture { raw: self.raw }.get_access()
    }

    /// Get the width of the texture.
    #[inline]
    pub fn width(&self) -> u32 {
        InternalTexture { raw: self.raw }.get_width()
    }

    /// Get the height of the texture.
    #[inline]
    pub fn height(&self) -> u32 {
        InternalTexture { raw: self.raw }.get_height()
    }

    /// Sets an additional color value multiplied into render copy operations.
    #[inline]
    pub fn set_color_mod(&mut self, red: u8, green: u8, blue: u8) {
        InternalTexture { raw: self.raw }.set_color_mod(red, green, blue)
    }

    /// Gets the additional color value multiplied into render copy operations.
    #[inline]
    pub fn color_mod(&self) -> (u8, u8, u8) {
        InternalTexture { raw: self.raw }.color_mod()
    }

    /// Sets an additional alpha value multiplied into render copy operations.
    #[inline]
    pub fn set_alpha_mod(&mut self, alpha: u8) {
        InternalTexture { raw: self.raw }.set_alpha_mod(alpha)
    }

    /// Gets the additional alpha value multiplied into render copy operations.
    #[inline]
    pub fn alpha_mod(&self) -> u8 {
        InternalTexture { raw: self.raw }.alpha_mod()
    }

    /// Sets the blend mode used for drawing operations (Fill and Line).
    #[inline]
    pub fn set_blend_mode(&mut self, blend: BlendMode) {
        InternalTexture { raw: self.raw }.set_blend_mode(blend)
    }

    /// Gets the blend mode used for texture copy operations.
    #[inline]
    pub fn blend_mode(&self) -> BlendMode {
        InternalTexture { raw: self.raw }.blend_mode()
    }

    /// Sets the scale mode for use when rendered.
    #[inline]
    pub fn set_scale_mode(&mut self, scale: ScaleMode) {
        InternalTexture { raw: self.raw }.set_scale_mode(scale)
    }

    /// Gets the scale mode for use when rendered.
    #[inline]
    pub fn scale_mode(&self) -> ScaleMode {
        InternalTexture { raw: self.raw }.scale_mode()
    }

    /// Updates the given texture rectangle with new pixel data.
    ///
    /// `pitch` is the number of bytes in a row of pixel data, including padding
    /// between lines
    ///
    /// * If `rect` is `None`, the entire texture is updated.
    #[inline]
    pub fn update<R>(
        &mut self,
        rect: R,
        pixel_data: &[u8],
        pitch: usize,
    ) -> Result<(), UpdateTextureError>
    where
        R: Into<Option<Rect>>,
    {
        InternalTexture { raw: self.raw }.update(rect, pixel_data, pitch)
    }

    /// Updates a rectangle within a planar YV12 or IYUV texture with new pixel data.
    #[inline]
    pub fn update_yuv<R>(
        &mut self,
        rect: R,
        y_plane: &[u8],
        y_pitch: usize,
        u_plane: &[u8],
        u_pitch: usize,
        v_plane: &[u8],
        v_pitch: usize,
    ) -> Result<(), UpdateTextureYUVError>
    where
        R: Into<Option<Rect>>,
    {
        InternalTexture { raw: self.raw }
            .update_yuv(rect, y_plane, y_pitch, u_plane, u_pitch, v_plane, v_pitch)
    }

    /// Locks the texture for **write-only** pixel access.
    /// The texture must have been created with streaming access.
    ///
    /// `F` is a function that is passed the write-only texture buffer,
    /// and the pitch of the texture (size of a row in bytes).
    /// # Remarks
    /// As an optimization, the pixels made available for editing don't
    /// necessarily contain the old texture data.
    /// This is a write-only operation, and if you need to keep a copy of the
    /// texture data you should do that at the application level.
    #[inline]
    pub fn with_lock<F, R, R2>(&mut self, rect: R2, func: F) -> Result<R, Error>
    where
        F: FnOnce(&mut [u8], usize) -> R,
        R2: Into<Option<Rect>>,
    {
        InternalTexture { raw: self.raw }.with_lock(rect, func)
    }

    // /// Binds an OpenGL/ES/ES2 texture to the current
    // /// context for use with when rendering OpenGL primitives directly.
    // #[inline]
    // pub unsafe fn gl_bind_texture(&mut self) -> (f32, f32) {
    //     InternalTexture { raw: self.raw }.gl_bind_texture()
    // }
    //
    // /// Unbinds an OpenGL/ES/ES2 texture from the current context.
    // #[inline]
    // pub unsafe fn gl_unbind_texture(&mut self) {
    //     InternalTexture { raw: self.raw }.gl_unbind_texture()
    // }

    // /// Binds and unbinds an OpenGL/ES/ES2 texture from the current context.
    // #[inline]
    // pub fn gl_with_bind<R, F: FnOnce(f32, f32) -> R>(&mut self, f: F) -> R {
    //     InternalTexture { raw: self.raw }.gl_with_bind(f)
    // }

    #[inline]
    // this can prevent introducing UB until
    // https://github.com/rust-lang/rust-clippy/issues/5953 is fixed
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub const fn raw(&self) -> *mut sys::render::SDL_Texture {
        self.raw
    }

    /// A convenience function for [`TextureCreator::create_texture_from_surface`].
    ///
    /// ```no_run
    /// use sdl3::pixels::PixelFormat;
    /// use sdl3::surface::Surface;
    /// use sdl3::render::{Canvas, Texture};
    /// use sdl3::video::Window;
    ///
    /// // We init systems.
    /// let sdl_context = sdl3::init().expect("failed to init SDL");
    /// let video_subsystem = sdl_context.video().expect("failed to get video context");
    ///
    /// // We create a window.
    /// let window = video_subsystem.window("sdl3 demo", 800, 600)
    ///     .build()
    ///     .expect("failed to build window");
    ///
    /// // We get the canvas from which we can get the `TextureCreator`.
    /// let mut canvas: Canvas<Window> = window.into_canvas();
    /// let texture_creator = canvas.texture_creator();
    ///
    /// let surface = Surface::new(512, 512, PixelFormat::RGB24).unwrap();
    /// let texture = Texture::from_surface(&surface, &texture_creator).unwrap();
    /// ```
    #[cfg(not(feature = "unsafe_textures"))]
    pub fn from_surface<'a, T>(
        surface: &Surface,
        texture_creator: &'a TextureCreator<T>,
    ) -> Result<Texture<'a>, TextureValueError> {
        texture_creator.create_texture_from_surface(surface)
    }

    /// A convenience function for [`TextureCreator::create_texture_from_surface`].
    ///
    /// ```no_run
    /// use sdl3::pixels::PixelFormat;
    /// use sdl3::surface::Surface;
    /// use sdl3::render::{Canvas, Texture};
    /// use sdl3::video::Window;
    ///
    /// // We init systems.
    /// let sdl_context = sdl3::init().expect("failed to init SDL");
    /// let video_subsystem = sdl_context.video().expect("failed to get video context");
    ///
    /// // We create a window.
    /// let window = video_subsystem.window("sdl3 demo", 800, 600)
    ///     .build()
    ///     .expect("failed to build window");
    ///
    /// // We get the canvas from which we can get the `TextureCreator`.
    /// let mut canvas: Canvas<Window> = window.into_canvas();
    /// let texture_creator = canvas.texture_creator();
    ///
    /// let surface = Surface::new(512, 512, PixelFormat::RGB24).unwrap();
    /// let texture = Texture::from_surface(&surface, &texture_creator).unwrap();
    /// ```
    #[cfg(feature = "unsafe_textures")]
    pub fn from_surface<T>(
        surface: &Surface,
        texture_creator: &TextureCreator<T>,
    ) -> Result<Texture, TextureValueError> {
        texture_creator.create_texture_from_surface(surface)
    }
}

#[cfg(feature = "unsafe_textures")]
impl Texture {
    /// Gets the texture's internal properties.
    #[inline]
    pub fn query(&self) -> TextureQuery {
        let internal = InternalTexture { raw: self.raw };
        TextureQuery {
            format: internal.get_format(),
            access: internal.get_access(),
            width: internal.get_width(),
            height: internal.get_height(),
        }
    }

    /// Get the format of the texture.
    #[inline]
    pub fn format(&self) -> PixelFormat {
        InternalTexture { raw: self.raw }.get_format()
    }

    /// Get the access of the texture.
    #[inline]
    pub fn access(&self) -> TextureAccess {
        InternalTexture { raw: self.raw }.get_access()
    }

    /// Get the width of the texture.
    #[inline]
    pub fn width(&self) -> u32 {
        InternalTexture { raw: self.raw }.get_width()
    }

    /// Get the height of the texture.
    #[inline]
    pub fn height(&self) -> u32 {
        InternalTexture { raw: self.raw }.get_height()
    }

    /// Sets an additional color value multiplied into render copy operations.
    #[inline]
    pub fn set_color_mod(&mut self, red: u8, green: u8, blue: u8) {
        InternalTexture { raw: self.raw }.set_color_mod(red, green, blue)
    }

    /// Gets the additional color value multiplied into render copy operations.
    #[inline]
    pub fn color_mod(&self) -> (u8, u8, u8) {
        InternalTexture { raw: self.raw }.color_mod()
    }

    /// Sets an additional alpha value multiplied into render copy operations.
    #[inline]
    pub fn set_alpha_mod(&mut self, alpha: u8) {
        InternalTexture { raw: self.raw }.set_alpha_mod(alpha)
    }

    /// Gets the additional alpha value multiplied into render copy operations.
    #[inline]
    pub fn alpha_mod(&self) -> u8 {
        InternalTexture { raw: self.raw }.alpha_mod()
    }

    /// Sets the blend mode used for drawing operations (Fill and Line).
    #[inline]
    pub fn set_blend_mode(&mut self, blend: BlendMode) {
        InternalTexture { raw: self.raw }.set_blend_mode(blend)
    }

    /// Gets the blend mode used for texture copy operations.
    #[inline]
    pub fn blend_mode(&self) -> BlendMode {
        InternalTexture { raw: self.raw }.blend_mode()
    }

    /// Updates the given texture rectangle with new pixel data.
    ///
    /// `pitch` is the number of bytes in a row of pixel data, including padding
    /// between lines
    ///
    /// * If `rect` is `None`, the entire texture is updated.
    #[inline]
    pub fn update<R>(
        &mut self,
        rect: R,
        pixel_data: &[u8],
        pitch: usize,
    ) -> Result<(), UpdateTextureError>
    where
        R: Into<Option<Rect>>,
    {
        InternalTexture { raw: self.raw }.update(rect, pixel_data, pitch)
    }

    /// Updates a rectangle within a planar YV12 or IYUV texture with new pixel data.
    #[inline]
    pub fn update_yuv<R>(
        &mut self,
        rect: R,
        y_plane: &[u8],
        y_pitch: usize,
        u_plane: &[u8],
        u_pitch: usize,
        v_plane: &[u8],
        v_pitch: usize,
    ) -> Result<(), UpdateTextureYUVError>
    where
        R: Into<Option<Rect>>,
    {
        InternalTexture { raw: self.raw }
            .update_yuv(rect, y_plane, y_pitch, u_plane, u_pitch, v_plane, v_pitch)
    }

    /// Locks the texture for **write-only** pixel access.
    /// The texture must have been created with streaming access.
    ///
    /// `F` is a function that is passed the write-only texture buffer,
    /// and the pitch of the texture (size of a row in bytes).
    /// # Remarks
    /// As an optimization, the pixels made available for editing don't
    /// necessarily contain the old texture data.
    /// This is a write-only operation, and if you need to keep a copy of the
    /// texture data you should do that at the application level.
    #[inline]
    pub fn with_lock<F, R, R2>(&mut self, rect: R2, func: F) -> Result<R, Error>
    where
        F: FnOnce(&mut [u8], usize) -> R,
        R2: Into<Option<Rect>>,
    {
        InternalTexture { raw: self.raw }.with_lock(rect, func)
    }

    // these are not supplied by SDL anymore
    // not sure if we should support them since we'd need to pull in OpenGL
    // /// Binds an OpenGL/ES/ES2 texture to the current
    // /// context for use with when rendering OpenGL primitives directly.
    // #[inline]
    // pub unsafe fn gl_bind_texture(&mut self) -> (f32, f32) {
    //     InternalTexture { raw: self.raw }.gl_bind_texture()
    // }
    //
    // /// Unbinds an OpenGL/ES/ES2 texture from the current context.
    // #[inline]
    // pub unsafe fn gl_unbind_texture(&mut self) {
    //     InternalTexture { raw: self.raw }.gl_unbind_texture()
    // }
    //
    // /// Binds and unbinds an OpenGL/ES/ES2 texture from the current context.
    // #[inline]
    // pub fn gl_with_bind<R, F: FnOnce(f32, f32) -> R>(&mut self, f: F) -> R {
    //     InternalTexture { raw: self.raw }.gl_with_bind(f)
    // }

    #[inline]
    // this can prevent introducing UB until
    // https://github.com/rust-lang/rust-clippy/issues/5953 is fixed
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub const fn raw(&self) -> *mut sys::render::SDL_Texture {
        self.raw
    }
}

#[derive(Copy, Clone)]
pub struct DriverIterator {
    length: i32,
    index: i32,
}

impl Iterator for DriverIterator {
    type Item = String;

    #[inline]
    #[doc(alias = "SDL_GetRenderDriver")]
    fn next(&mut self) -> Option<String> {
        if self.index >= self.length {
            None
        } else {
            let result = unsafe { sys::render::SDL_GetRenderDriver(self.index) };
            self.index += 1;

            Some(
                unsafe { CStr::from_ptr(result) }
                    .to_string_lossy()
                    .into_owned(),
            )
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let l = self.length as usize;
        (l, Some(l))
    }
}

impl ExactSizeIterator for DriverIterator {}

/// Gets an iterator of all render drivers compiled into the SDL2 library.
#[inline]
#[doc(alias = "SDL_GetNumRenderDrivers")]
pub fn drivers() -> DriverIterator {
    // This function is thread-safe and doesn't require the video subsystem to be initialized.
    // The list of drivers are read-only and statically compiled into SDL2, varying by platform.

    // SDL_GetNumRenderDrivers can never return a negative value.
    DriverIterator {
        length: unsafe { sys::render::SDL_GetNumRenderDrivers() },
        index: 0,
    }
}
