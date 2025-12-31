use std::fs::File;
use std::io::Write;
use zip::{ZipWriter, CompressionMethod, AesMode};
use zip::write::SimpleFileOptions;

fn main() {
    let file = File::create("C:/Dev/Repos/hobby/time-locker/test_encrypted.zip").unwrap();
    let mut zip = ZipWriter::new(file);
    
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .with_aes_encryption(AesMode::Aes256, "mysecretpassword");
    
    zip.start_file("secret.txt", options).unwrap();
    zip.write_all(b"This is secret content that should be encrypted!").unwrap();
    zip.finish().unwrap();
    
    println!("Created test_encrypted.zip - try opening without password");
}
