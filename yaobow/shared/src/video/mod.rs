// The Bik decoder is backed by ffmpeg, which is not cross-compiled for Switch
// yet, so that target registers no decoders and simply plays no movies.
#[cfg(not(switch))]
mod ffmpeg;

#[cfg(not(switch))]
pub fn register_opengb_video_decoders() {
    use radiance::video::{Codec, register_video_decoder};
    register_video_decoder(Codec::Bik, ffmpeg::VideoStreamFFmpeg::create);
}

#[cfg(switch)]
pub fn register_opengb_video_decoders() {}
