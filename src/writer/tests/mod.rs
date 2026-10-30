mod backdrops;
mod borders;
mod clips;
mod filters;

fn render(size: [u16; 2], draw: impl FnOnce(&mut krilla::surface::Surface<'_>)) -> Vec<u8> {
    let mut document = krilla::Document::new();
    let mut page = document.start_page_with(
        krilla::page::PageSettings::from_wh(f32::from(size[0]), f32::from(size[1])).unwrap(),
    );
    let mut surface = page.surface();
    draw(&mut surface);
    surface.finish();
    page.finish();
    let pdf = hayro::hayro_syntax::Pdf::new(document.finish().unwrap()).unwrap();
    let pixels = hayro::render(
        &pdf.pages()[0],
        &hayro::RenderCache::new(),
        &hayro::hayro_interpret::InterpreterSettings::default(),
        &hayro::RenderSettings::default(),
        &hayro::PixmapSettings {
            x_scale: 1.0,
            y_scale: 1.0,
            bg_color: hayro::vello_cpu::color::palette::css::TRANSPARENT,
        },
    );
    assert_eq!([pixels.width(), pixels.height()], size);
    pixels.data_as_u8_slice().to_vec()
}
