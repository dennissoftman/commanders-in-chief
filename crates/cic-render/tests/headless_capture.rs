use cic_render::{HeadlessRenderer, Pose, RenderError};

#[test]
fn synthetic_pose_capture_matches_completion_hash() {
    let renderer = match pollster::block_on(HeadlessRenderer::new()) {
        Ok(renderer) => renderer,
        Err(RenderError::RequestAdapter(error)) => {
            eprintln!("skipping GPU capture without a headless adapter: {error}");
            return;
        }
        Err(error) => panic!("initializing headless renderer: {error}"),
    };
    let capture = renderer
        .capture_triangle(64, 64, Pose::translation(0.25, 0.0).expect("finite pose"))
        .expect("headless capture");
    assert!(matches!(
        renderer.capture_triangle(4_097, 1, Pose::IDENTITY),
        Err(RenderError::CaptureTooLarge)
    ));
    let expected = include_str!("fixtures/synthetic-pose.rgba.sha256").trim();
    assert_eq!(capture.sha256(), expected);
}
