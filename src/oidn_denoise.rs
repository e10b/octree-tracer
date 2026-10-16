//! Intel Open Image Denoise 2. The RT filter cleans a path-traced frame from
//! its color, albedo, and shading normal. The device is created once.

pub struct Denoiser {
    filter: oidn::RayTracing<'static>,
    width: usize,
    height: usize,
}

impl Denoiser {
    pub fn new(width: u32, height: u32) -> Option<Self> {
        let device = shared_device()?;
        let mut filter = oidn::RayTracing::try_new(device).ok()?;
        filter
            .image_dimensions(width as usize, height as usize)
            .hdr(true)
            .clean_aux(true)
            .filter_quality(oidn::Quality::Balanced);
        Some(Self {
            filter,
            width: width as usize,
            height: height as usize,
        })
    }

    /// `color`, `albedo`, and `normal` are packed RGB. Albedo is in 0..1 and
    /// the normal is the shading normal. The gbuffers are stable, so the
    /// auxiliary images are marked clean.
    pub fn denoise(
        &mut self,
        width: u32,
        height: u32,
        color: &[f32],
        albedo: &[f32],
        normal: &[f32],
    ) -> Result<Vec<f32>, oidn::Error> {
        let width = width as usize;
        let height = height as usize;
        if self.width != width || self.height != height {
            self.filter.image_dimensions(width, height);
            self.width = width;
            self.height = height;
        }
        self.filter.albedo_normal(albedo, normal);
        let mut output = vec![0.0; color.len()];
        self.filter.filter(color, &mut output)?;
        Ok(output)
    }
}

fn shared_device() -> Option<&'static oidn::Device> {
    static SLOT: std::sync::Mutex<Option<usize>> = std::sync::Mutex::new(None);
    let mut slot = SLOT.lock().ok()?;
    if slot.is_none() {
        match oidn::Device::new() {
            Ok(device) => {
                println!("OIDN 2 device ready");
                let leaked: &'static oidn::Device = Box::leak(Box::new(device));
                *slot = Some(leaked as *const oidn::Device as usize);
            }
            Err(err) => {
                println!("OIDN 2 unavailable: {err}");
                *slot = Some(0);
            }
        }
    }
    let addr = (*slot)?;
    if addr == 0 {
        None
    } else {
        Some(unsafe { &*(addr as *const oidn::Device) })
    }
}

pub fn unpack_rgb(padded: &[u8], width: u32, height: u32, row_bytes: u32) -> Vec<f32> {
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        let row = &padded[(y * row_bytes) as usize..];
        for x in 0..width {
            let at = (x * 8) as usize;
            rgb.push(half_to_f32(u16::from_le_bytes([row[at], row[at + 1]])));
            rgb.push(half_to_f32(u16::from_le_bytes([row[at + 2], row[at + 3]])));
            rgb.push(half_to_f32(u16::from_le_bytes([row[at + 4], row[at + 5]])));
        }
    }
    rgb
}

pub fn pack_rgba(rgb: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(rgb.len() / 3 * 8);
    for pixel in rgb.chunks_exact(3) {
        for channel in pixel {
            bytes.extend(f32_to_half(*channel).to_le_bytes());
        }
        bytes.extend(f32_to_half(1.0).to_le_bytes());
    }
    bytes
}

fn half_to_f32(bits: u16) -> f32 {
    half::f16::from_bits(bits).to_f32()
}

fn f32_to_half(value: f32) -> u16 {
    half::f16::from_f32(value).to_bits()
}

#[cfg(test)]
mod tests {
    #[test]
    fn half_roundtrip_keeps_hdr_color() {
        for value in [0.0, 1.0, -1.0, 0.5, 2.0, 12.0] {
            let back = super::half_to_f32(super::f32_to_half(value));
            assert!((back - value).abs() < 0.01, "{value} -> {back}");
        }
    }

    #[test]
    fn unpack_reads_rgb_and_skips_the_row_padding() {
        let pixel = super::pack_rgba(&[0.25, 0.5, 1.0]);
        let mut padded = pixel.clone();
        padded.extend([0u8; 8]);
        let rgb = super::unpack_rgb(&padded, 1, 1, 16);
        assert!((rgb[0] - 0.25).abs() < 0.01);
        assert!((rgb[1] - 0.5).abs() < 0.01);
        assert!((rgb[2] - 1.0).abs() < 0.01);
    }

    #[test]
    fn oidn_denoises_a_flat_color() {
        let mut denoiser = super::Denoiser::new(4, 4).expect("OIDN device");
        let color = vec![0.4, 0.2, 0.1].repeat(16);
        let albedo = vec![0.5, 0.5, 0.5].repeat(16);
        let normal = vec![0.0, 1.0, 0.0].repeat(16);
        let output = denoiser
            .denoise(4, 4, &color, &albedo, &normal)
            .expect("filter");
        assert_eq!(output.len(), color.len());
        assert!(output.iter().any(|channel| *channel > 0.0));
    }
}
