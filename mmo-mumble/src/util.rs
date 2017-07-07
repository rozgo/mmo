#![allow(dead_code)]



fn opus_analyze(opus_data : &Vec<u8>) {
    println!("======================================================");
    if let Ok(_) = opus::packet::parse(&opus_data) {
        println!("opus::packet::parse: ok");
    }
    if let Ok(p) = opus::packet::get_nb_frames(&opus_data) {
        println!("opus::packet::get_nb_frames: {:?}", p);
    }
    if let Ok(p) = opus::packet::get_nb_samples(&opus_data, 16000) {
        println!("opus::packet::get_nb_samples: {:?}", p);
    }
    if let Ok(p) = opus::packet::get_samples_per_frame(&opus_data, 16000) {
        println!("opus::packet::get_samples_per_frame: {:?}", p);
    }
    if let Ok(p) = opus::packet::get_bandwidth(&opus_data) {
        println!("opus::packet::get_bandwidth: {:?}", p);
    }
    if let Ok(p) = opus::packet::get_nb_channels(&opus_data) {
        println!("opus::packet::get_nb_channels: {:?}", p);
    }
    println!("======================================================");
}