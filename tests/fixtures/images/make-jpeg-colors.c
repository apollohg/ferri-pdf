#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <jpeglib.h>

enum { WIDTH = 64, HEIGHT = 48, MAX_CHANNELS = 4, QUALITY = 97 };

int main(int argc, char **argv) {
    if (argc != 3 || (strcmp(argv[2], "rgb") && strcmp(argv[2], "ycck"))) {
        fprintf(stderr, "usage: make-jpeg-colors OUTPUT.jpg rgb|ycck\n");
        return EXIT_FAILURE;
    }
    const int is_rgb = !strcmp(argv[2], "rgb");
    FILE *output = fopen(argv[1], "wb");
    if (!output) { perror("JPEG output"); return EXIT_FAILURE; }
    struct jpeg_compress_struct encoder;
    struct jpeg_error_mgr error;
    encoder.err = jpeg_std_error(&error);
    jpeg_create_compress(&encoder);
    jpeg_stdio_dest(&encoder, output);
    encoder.image_width = WIDTH;
    encoder.image_height = HEIGHT;
    encoder.input_components = is_rgb ? 3 : 4;
    encoder.in_color_space = is_rgb ? JCS_RGB : JCS_CMYK;
    jpeg_set_defaults(&encoder);
    jpeg_set_colorspace(&encoder, is_rgb ? JCS_RGB : JCS_YCCK);
    jpeg_set_quality(&encoder, QUALITY, TRUE);
    jpeg_start_compress(&encoder, TRUE);
    JSAMPLE row[WIDTH * MAX_CHANNELS];
    while (encoder.next_scanline < HEIGHT) {
        const unsigned y = encoder.next_scanline;
        for (unsigned x = 0; x < WIDTH; ++x) {
            const unsigned offset = x * encoder.input_components;
            row[offset] = x * MAXJSAMPLE / (WIDTH - 1);
            row[offset + 1] = y * MAXJSAMPLE / (HEIGHT - 1);
            row[offset + 2] = (x + y) * MAXJSAMPLE / (WIDTH + HEIGHT - 2);
            if (!is_rgb) row[offset + 3] = (x * y) * MAXJSAMPLE / ((WIDTH - 1) * (HEIGHT - 1));
        }
        JSAMPROW rows[] = { row };
        jpeg_write_scanlines(&encoder, rows, 1);
    }
    jpeg_finish_compress(&encoder);
    jpeg_destroy_compress(&encoder);
    if (fclose(output)) { perror("JPEG output"); return EXIT_FAILURE; }
    return EXIT_SUCCESS;
}
