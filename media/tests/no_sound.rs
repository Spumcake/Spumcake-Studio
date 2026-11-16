//! Its own test program, so setting the variable races with no other test.

/// Catches tests and screenshot scenarios that play sound on the owner's
/// computer: with `INPAINTER_NO_SOUND` set, `Player::open` opens no output
/// and its clock moves only as the test advances it.
#[test]
fn no_sound_opens_a_silent_player() {
    // SAFETY: this test program has one test and no other thread reading
    // the environment while it is set.
    unsafe { std::env::set_var("INPAINTER_NO_SOUND", "1") };
    let player = media::Player::open(48_000, (24, 1));
    assert!(!player.has_output());
    assert!(player.problem().is_none(), "a silent player reported a refused device");
    player.play_from(0);
    player.advance(1000.0);
    assert!((player.clock().unwrap() - 1.0).abs() < 1e-9);
}
