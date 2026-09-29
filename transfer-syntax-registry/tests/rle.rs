//! Test suite for RLE lossless pixel data reading and writing
#![cfg(feature = "rle")]

mod adapters;

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use adapters::TestDataObject;
use dicom_core::value::PixelFragmentSequence;
use dicom_encoding::{Codec, adapters::PixelDataReader};
use dicom_transfer_syntax_registry::entries::RLE_LOSSLESS;

fn read_data_piece(test_file: impl AsRef<Path>, offset: u64, length: usize) -> Vec<u8> {
    let mut file = File::open(test_file).unwrap();
    let mut buf = vec![0; length];
    file.seek(SeekFrom::Start(offset)).unwrap();
    file.read_exact(&mut buf).unwrap();
    buf
}

fn check_u16_rgb_pixel(pixels: &[u8], columns: u16, x: u16, y: u16, expected_pixel: [u16; 3]) {
    let i = (y as usize * columns as usize * 3 + x as usize * 3) * 2;
    let got = [
        u16::from_le_bytes([pixels[i], pixels[i + 1]]),
        u16::from_le_bytes([pixels[i + 2], pixels[i + 3]]),
        u16::from_le_bytes([pixels[i + 4], pixels[i + 5]]),
    ];
    assert_eq!(
        got, expected_pixel,
        "pixel sample mismatch at ({x}, {y}): {got:?} vs {expected_pixel:?}"
    );
}

fn check_i16_monochrome_pixel(pixels: &[u8], columns: u16, x: u16, y: u16, expected: i16) {
    let i = (y as usize * columns as usize + x as usize) * 2;
    let got = i16::from_le_bytes([pixels[i], pixels[i + 1]]);
    assert_eq!(
        got, expected,
        "pixel sample mismatch at ({x}, {y}): {got:?} vs {expected:?}"
    );
}

#[test]
fn read_rle_1() {
    let test_file = dicom_test_files::path("WG04/RLE/CT1_RLE").unwrap();

    // manually fetch the pixel data fragment from the file:

    // PixelData offset: 0x18f6
    // first fragment item offset: 0x1902
    // first fragment size: 4
    // second fragment item offset: 0x190e
    // second fragment size: 248330

    // single fragment found in file data offset 0x1916, 248330 bytes
    let buf = read_data_piece(test_file, 0x1916, 248330);

    // create test object
    let obj = TestDataObject {
        // RLE lossless
        ts_uid: "1.2.840.10008.1.2.5".to_string(),
        rows: 512,
        columns: 512,
        bits_allocated: 16,
        bits_stored: 16,
        samples_per_pixel: 1,
        photometric_interpretation: "MONOCHROME2",
        number_of_frames: 1,
        flat_pixel_data: None,
        pixel_data_sequence: Some(PixelFragmentSequence::new(vec![], vec![buf])),
    };

    // instantiate RLE lossless adapter

    let Codec::EncapsulatedPixelData(Some(adapter), _) = RLE_LOSSLESS.codec() else {
        panic!("RLE lossless pixel data reader not found")
    };

    let mut dest = vec![];

    // decode the whole image (1 frame)

    adapter
        .decode(&obj, &mut dest)
        .expect("RLE frame decoding failed");

    // inspect the result
    assert_eq!(dest.len(), 512 * 512 * 2);

    // check a few known pixels
    check_i16_monochrome_pixel(&dest, 512, 16, 16, -2_000);

    check_i16_monochrome_pixel(&dest, 512, 255, 255, 980);

    check_i16_monochrome_pixel(&dest, 512, 342, 336, 188);

    // decode a single frame
    let mut dest2 = vec![];
    adapter
        .decode_frame(&obj, 0, &mut dest2)
        .expect("RLE frame decoding failed");

    // the outcome should be the same
    assert_eq!(dest, dest2);
}

#[test]
fn read_rle_2() {
    let test_file = dicom_test_files::path("pydicom/SC_rgb_rle_16bit.dcm").unwrap();

    // manually fetch the pixel data fragment from the file:
    // PixelData offset: 0x51a
    // first fragment item offset: 0x526
    // first fragment size: 0
    // second fragment item offset: 0x52e
    // second fragment size: 1264

    // single fragment found in file data offset 0x536, 1264 bytes
    let buf = read_data_piece(test_file, 0x536, 1_264);

    // create test object
    let obj = TestDataObject {
        // RLE lossless
        ts_uid: "1.2.840.10008.1.2.5".to_string(),
        rows: 100,
        columns: 100,
        bits_allocated: 16,
        bits_stored: 16,
        samples_per_pixel: 3,
        photometric_interpretation: "RGB",
        number_of_frames: 1,
        flat_pixel_data: None,
        pixel_data_sequence: Some(PixelFragmentSequence::new(vec![], vec![buf])),
    };

    // instantiate RLE lossless adapter

    let Codec::EncapsulatedPixelData(Some(adapter), _) = RLE_LOSSLESS.codec() else {
        panic!("RLE lossless pixel data reader not found")
    };

    let mut dest = vec![];

    // decode the whole image (1 frame)

    adapter
        .decode(&obj, &mut dest)
        .expect("RLE frame decoding failed");

    // inspect the result
    assert_eq!(dest.len(), 100 * 100 * 2 * 3);

    // check a few known pixels
    check_u16_rgb_pixel(&dest, 100, 0, 0, [0xFFFF, 0, 0]);

    check_u16_rgb_pixel(&dest, 100, 99, 19, [0xFFFF, 32_896, 32_896]);

    check_u16_rgb_pixel(&dest, 100, 54, 65, [0, 0, 0]);

    check_u16_rgb_pixel(&dest, 100, 10, 95, [0xFFFF, 0xFFFF, 0xFFFF]);
}

/// Encode one RLE frame from segments, each as a single PackBits literal run.
fn rle_frame(segments: &[Vec<u8>]) -> Vec<u8> {
    let mut frame = vec![0; 64];
    frame[0..4].copy_from_slice(&(segments.len() as u32).to_le_bytes());
    for (i, segment) in segments.iter().enumerate() {
        let offset = frame.len() as u32;
        frame[4 + 4 * i..8 + 4 * i].copy_from_slice(&offset.to_le_bytes());
        assert!(!segment.is_empty() && segment.len() <= 128);
        frame.push(segment.len() as u8 - 1);
        frame.extend_from_slice(segment);
    }
    frame
}

fn rle_object(
    bits_allocated: u16,
    samples_per_pixel: u16,
    columns: u16,
    frames: Vec<Vec<u8>>,
) -> TestDataObject {
    TestDataObject {
        ts_uid: "1.2.840.10008.1.2.5".to_string(),
        rows: 1,
        columns,
        bits_allocated,
        bits_stored: bits_allocated,
        samples_per_pixel,
        photometric_interpretation: if samples_per_pixel == 3 {
            "RGB"
        } else {
            "MONOCHROME2"
        },
        number_of_frames: frames.len() as u32,
        flat_pixel_data: None,
        pixel_data_sequence: Some(PixelFragmentSequence::new(vec![], frames)),
    }
}

fn decode_all(obj: &TestDataObject) -> Vec<u8> {
    let Codec::EncapsulatedPixelData(Some(adapter), _) = RLE_LOSSLESS.codec() else {
        panic!("RLE lossless pixel data reader not found")
    };
    let mut dest = vec![];
    adapter.decode(obj, &mut dest).expect("RLE decoding failed");
    dest
}

fn decode_one(obj: &TestDataObject, frame: u32) -> Vec<u8> {
    let Codec::EncapsulatedPixelData(Some(adapter), _) = RLE_LOSSLESS.codec() else {
        panic!("RLE lossless pixel data reader not found")
    };
    let mut dest = vec![];
    adapter
        .decode_frame(obj, frame, &mut dest)
        .expect("RLE frame decoding failed");
    dest
}

#[test]
fn read_rle_8bit_monochrome_sample_positions() {
    let obj = rle_object(
        8,
        1,
        4,
        vec![
            rle_frame(&[vec![1, 2, 3, 4]]),
            rle_frame(&[vec![5, 6, 7, 8]]),
        ],
    );
    assert_eq!(decode_all(&obj), [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(decode_one(&obj, 0), [1, 2, 3, 4]);
    assert_eq!(decode_one(&obj, 1), [5, 6, 7, 8]);
}

#[test]
fn read_rle_16bit_monochrome_sample_positions() {
    // samples 0x0102 and 0x0304: the MSB segment comes first
    let obj = rle_object(
        16,
        1,
        2,
        vec![rle_frame(&[vec![0x01, 0x03], vec![0x02, 0x04]])],
    );
    let expected = [0x02, 0x01, 0x04, 0x03];
    assert_eq!(decode_all(&obj), expected);
    assert_eq!(decode_one(&obj, 0), expected);
}

#[test]
fn read_rle_8bit_rgb_sample_positions() {
    let obj = rle_object(
        8,
        3,
        2,
        vec![rle_frame(&[vec![1, 4], vec![2, 5], vec![3, 6]])],
    );
    let expected = [1, 2, 3, 4, 5, 6];
    assert_eq!(decode_all(&obj), expected);
    assert_eq!(decode_one(&obj, 0), expected);
}

#[test]
fn read_rle_16bit_rgb_sample_positions() {
    // pixels (0x0102, 0x0304, 0x0506) and (0x0708, 0x090A, 0x0B0C)
    let obj = rle_object(
        16,
        3,
        2,
        vec![rle_frame(&[
            vec![0x01, 0x07],
            vec![0x02, 0x08],
            vec![0x03, 0x09],
            vec![0x04, 0x0A],
            vec![0x05, 0x0B],
            vec![0x06, 0x0C],
        ])],
    );
    let expected = [
        0x02, 0x01, 0x04, 0x03, 0x06, 0x05, 0x08, 0x07, 0x0A, 0x09, 0x0C, 0x0B,
    ];
    assert_eq!(decode_all(&obj), expected);
    assert_eq!(decode_one(&obj, 0), expected);
    check_u16_rgb_pixel(&expected, 2, 1, 0, [0x0708, 0x090A, 0x0B0C]);
}
