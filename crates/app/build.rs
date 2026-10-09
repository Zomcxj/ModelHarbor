fn main() {
    let python = "python";
    if std::path::Path::new("assets/icon.png").exists() {
        let output = std::process::Command::new(python)
            .args([
                "-c",
                r#"
from PIL import Image
img = Image.open("assets/icon.png").convert("RGBA")
img.thumbnail((256, 256), Image.LANCZOS)
img.save("assets/icon.ico")
w, h = img.size
if w != 256 or h != 256:
    new_img = Image.new("RGBA", (256, 256), (0, 0, 0, 0))
    new_img.paste(img, ((256 - w) // 2, (256 - h) // 2))
    img = new_img
img = img.resize((256, 256), Image.LANCZOS)
with open("assets/icon_rgba.bin", "wb") as f:
    f.write(img.tobytes())
print("icon ready:", len(img.tobytes()))
"#,
            ])
            .output();
        match output {
            Ok(output) if output.status.success() => {}
            Ok(output) => eprintln!(
                "icon.png conversion failed, keeping assets/icon.ico as-is: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
            Err(e) => eprintln!(
                "python unavailable ({e}), keeping assets/icon.ico as-is; \
                 install Python with pillow to regenerate it from assets/icon.png"
            ),
        }
    } else if !std::path::Path::new("assets/icon_rgba.bin").exists() {
        let (big_w, big_h) = (256usize, 256usize);
        let mut big_rgba = vec![0u8; big_w * big_h * 4];
        let bcx = big_w as f32 / 2.0;
        let bcy = big_h as f32 / 2.0;
        let outer_r = 108.0;
        let inner_r = 48.0;
        for y in 0..big_h {
            for x in 0..big_w {
                let dx = x as f32 - bcx;
                let dy = y as f32 - bcy;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist <= outer_r && dist >= inner_r {
                    let idx = (y * big_w + x) * 4;
                    big_rgba[idx] = 255;
                    big_rgba[idx + 1] = 255;
                    big_rgba[idx + 2] = 255;
                    big_rgba[idx + 3] = 255;
                }
            }
        }
        std::fs::write("assets/icon_rgba.bin", &big_rgba).unwrap();
    }

    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon.ico");
    res.compile().unwrap();
}
