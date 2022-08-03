//! Head-Related Transfer Function (HRTF) module. Provides all needed types and methods for HRTF rendering.
//!
//! # Overview
//!
//! HRTF stands for [Head-Related Transfer Function](https://en.wikipedia.org/wiki/Head-related_transfer_function)
//! and can work only with spatial sounds. For each of such sound source after it was processed by HRTF you can
//! definitely tell from which locationsound came from. In other words HRTF improves perception of sound to
//! the level of real life.
//!
//! # HRIR Spheres
//!
//! This library uses Head-Related Impulse Response (HRIR) spheres to create HRTF spheres. HRTF sphere is a set of
//! points in 3D space which are connected into a mesh forming triangulated sphere. Each point contains spectrum
//! for left and right ears which will be used to modify samples from each spatial sound source to create binaural
//! sound. HRIR spheres can be found [here](https://github.com/mrDIMAS/hrir_sphere_builder/tree/master/hrtf_base/IRCAM)
//!
//! # Usage
//!
//! To use HRTF you need to change default renderer to HRTF renderer like so:
//!
//! ```no_run
//! use fyrox_sound::context::{self, SoundContext};
//! use fyrox_sound::renderer::hrtf::{HrtfRenderer};
//! use fyrox_sound::renderer::Renderer;
//! use std::path::Path;
//! use hrtf::HrirSphere;
//!
//! fn use_hrtf(context: &mut SoundContext) {
//!     // IRC_1002_C.bin is HRIR sphere in binary format, can be any valid HRIR sphere
//!     // from base mentioned above.
//!     let hrir_sphere = HrirSphere::from_file("examples/data/IRC_1002_C.bin", context::SAMPLE_RATE).unwrap();
//!
//!     context.state().set_renderer(Renderer::HrtfRenderer(HrtfRenderer::new(hrir_sphere)));
//! }
//! ```
//!
//! # Performance
//!
//! HRTF is `heavy`. Usually it 4-5 slower than default renderer, this is essential because HRTF requires some heavy
//! math (fast Fourier transform, convolution, etc.). On Ryzen 1700 it takes 400-450 μs (0.4 - 0.45 ms) per source.
//! In most cases this is ok, engine works in separate thread and it has around 100 ms to prepare new portion of
//! samples for output device.
//!
//! # Known problems
//!
//! This renderer still suffers from small audible clicks in very fast moving sounds, clicks sounds more like
//! "buzzing" - it is due the fact that hrtf is different from frame to frame which gives "bumps" in amplitude
//! of signal because of phase shift each impulse response have. This can be fixed by short cross fade between
//! small amount of samples from previous frame with same amount of frames of current as proposed in
//! [here](http://csoundjournal.com/issue9/newHRTFOpcodes.html)
//!
//! Clicks can be reproduced by using clean sine wave of 440 Hz on some source moving around listener.

use crate::{
    context::{self, DistanceModel, SoundContext},
    listener::Listener,
    renderer::render_source_2d_only,
    source::SoundSource,
};
use fyrox_core::{
    inspect::{Inspect, PropertyInfo},
    reflect::Reflect,
    visitor::{Visit, VisitResult, Visitor},
};
use hrtf::HrirSphere;
use std::{fmt::Debug, path::PathBuf};

/// See module docs.
#[derive(Clone, Debug, Default, Inspect, Reflect)]
pub struct HrtfRenderer {
    hrir_path: PathBuf,
    #[inspect(skip)]
    #[reflect(hidden)]
    processor: Option<hrtf::HrtfProcessor>,
}

impl Visit for HrtfRenderer {
    fn visit(&mut self, name: &str, visitor: &mut Visitor) -> VisitResult {
        let mut region = visitor.enter_region(name)?;

        self.hrir_path.visit("ResourcePath", &mut region)?;

        drop(region);

        if visitor.is_reading() {
            self.processor = Some(hrtf::HrtfProcessor::new(
                HrirSphere::from_file(&self.hrir_path, context::SAMPLE_RATE).unwrap(),
                SoundContext::HRTF_INTERPOLATION_STEPS,
                SoundContext::HRTF_BLOCK_LEN,
            ));
        }

        Ok(())
    }
}

impl HrtfRenderer {
    /// Creates new HRTF renderer using specified HRTF sphere. See module docs for more info.
    pub fn new(hrir_sphere: hrtf::HrirSphere) -> Self {
        Self {
            hrir_path: hrir_sphere.source().to_path_buf(),
            processor: Some(hrtf::HrtfProcessor::new(
                hrir_sphere,
                SoundContext::HRTF_INTERPOLATION_STEPS,
                SoundContext::HRTF_BLOCK_LEN,
            )),
        }
    }

    pub(crate) fn render_source(
        &mut self,
        source: &mut SoundSource,
        listener: &Listener,
        distance_model: DistanceModel,
        out_buf: &mut [(f32, f32)],
    ) {
        // Render as 2D first with k = (1.0 - spatial_blend).
        render_source_2d_only(source, out_buf);

        // Then add HRTF part with k = spatial_blend
        let new_distance_gain =
            source.spatial_blend() * source.calculate_distance_gain(listener, distance_model);
        let new_sampling_vector = source.calculate_sampling_vector(listener);

        self.processor
            .as_mut()
            .unwrap()
            .process_samples(hrtf::HrtfContext {
                source: &source.frame_samples,
                output: out_buf,
                new_sample_vector: hrtf::Vec3::new(
                    new_sampling_vector.x,
                    new_sampling_vector.y,
                    new_sampling_vector.z,
                ),
                prev_sample_vector: hrtf::Vec3::new(
                    source.prev_sampling_vector.x,
                    source.prev_sampling_vector.y,
                    source.prev_sampling_vector.z,
                ),
                prev_left_samples: &mut source.prev_left_samples,
                prev_right_samples: &mut source.prev_right_samples,
                prev_distance_gain: source.prev_distance_gain.unwrap_or(new_distance_gain),
                new_distance_gain,
            });

        source.prev_sampling_vector = new_sampling_vector;
        source.prev_distance_gain = Some(new_distance_gain);
    }
}
