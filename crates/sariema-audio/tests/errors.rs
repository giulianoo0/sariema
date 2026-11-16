// SPDX-License-Identifier: Apache-2.0

use sariema_audio::AudioError;

#[test]
fn each_error_has_its_own_stable_code() {
    let codes = [
        (AudioError::Io("x".into()), "E_IO"),
        (AudioError::Decode("x".into()), "E_AUDIO_DECODE"),
        (
            AudioError::UnknownSound("x".into()),
            "E_AUDIO_UNKNOWN_SOUND",
        ),
        (AudioError::NotLoaded("x".into()), "E_AUDIO_NOT_LOADED"),
        (AudioError::QueueFull, "E_AUDIO_QUEUE_FULL"),
        (AudioError::NotFinite("gain"), "E_AUDIO_NOT_FINITE"),
        (AudioError::Settings("x".into()), "E_SCHEMA"),
        (
            AudioError::Device("no output device".into()),
            "E_AUDIO_DEVICE",
        ),
    ];
    for (error, code) in codes {
        assert_eq!(error.code(), code, "{error}");
    }
}
