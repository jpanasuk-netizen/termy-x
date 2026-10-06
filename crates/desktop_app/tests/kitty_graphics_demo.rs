#![cfg(unix)]

use std::{fs, path::Path, process::Command};

const APC_START: &[u8] = b"\x1b_G";
const APC_END: &[u8] = b"\x1b\\";
const CHUNK_SIZE: usize = 4096;

#[test]
fn demo_script_transmits_the_complete_bundled_image() {
    let package_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = package_dir.join("examples/kitty_graphics_demo.sh");
    let image = fs::read(package_dir.join("examples/assets/kitty-demo.png")).unwrap();

    let output = Command::new("bash").arg(script).output().unwrap();
    assert!(output.status.success());
    assert!(
        find(&output.stdout, b",i=").is_none(),
        "the repeatable demo must not replace a fixed image id"
    );
    assert!(
        find(&output.stdout, b",p=").is_none(),
        "the repeatable demo must not replace a fixed placement id"
    );

    let (payload_len, command_count, final_chunk_count) = inspect_stream(&output.stdout);
    let expected_payload_len = image.len().div_ceil(3) * 4;
    assert_eq!(payload_len, expected_payload_len);
    assert_eq!(command_count, expected_payload_len.div_ceil(CHUNK_SIZE));
    assert_eq!(final_chunk_count, 1);
}

fn inspect_stream(stream: &[u8]) -> (usize, usize, usize) {
    let mut cursor = 0;
    let mut payload_len = 0;
    let mut command_count = 0;
    let mut final_chunk_count = 0;

    while let Some(start) = find(&stream[cursor..], APC_START) {
        let body_start = cursor + start + APC_START.len();
        let end = find(&stream[body_start..], APC_END).expect("unterminated Kitty command");
        let body_end = body_start + end;
        let body = &stream[body_start..body_end];
        let separator = body
            .iter()
            .position(|byte| *byte == b';')
            .expect("Kitty command without payload separator");
        let control = &body[..separator];
        if control.starts_with(b"a=T,") || control.starts_with(b"m=") {
            payload_len += body.len() - separator - 1;
            command_count += 1;
            final_chunk_count += usize::from(control.ends_with(b"m=0"));
        }
        cursor = body_end + APC_END.len();
    }

    (payload_len, command_count, final_chunk_count)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
