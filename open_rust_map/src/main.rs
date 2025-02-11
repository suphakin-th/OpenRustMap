use osmpbfreader::{OsmPbfReader, Blob};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::open("data/thailand.pbf")?;
    let mut reader = PbfReader::new(file);

    while let Some(blob) = reader.next_blob()? {
        match blob {
            Blob::OSMData(data) => {
                // Process OSM data here
                println!("OSM data: {:?}", data);
            }
            Blob::OSMHeader(header) => {
                // Process OSM header here
                println!("OSM header: {:?}", header);
            }
            _ => {
                // Ignore other blob types
            }
        }
    }
    Ok(())
}

