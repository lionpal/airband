//! A picture in the chat, fetched from his computer in parts and kept on the device.
use super::*;

impl Radio {
    /// A picture, as a JPEG file on the device: fetched from the tower the first time and kept, so it is there
    /// at once afterwards and without a connection. Returns the file's path. Blocks: call it off the main thread.
    pub(crate) fn picture(&self, id: String) -> Result<String, RadioError> {
        // a picture's id is the hex its computer gave it: anything else names no picture there
        if !airband_core::encode::is_hex(&id, 1..=128) {
            return Err(RadioError::gone("picture"));
        }
        let file = self.store.pictures().join(format!("{id}.jpg"));
        if file.is_file() {
            return Ok(file.display().to_string());
        }
        let (mut bytes, mut from) = (Vec::new(), 0u32);
        loop {
            let part = self.link.call::<at::Picture>(Some(&id), &wire::PartQuery { from: from as usize }, &Nothing {})?;
            bytes.extend(airband_core::encode::unbase64(&part.data).ok_or_else(|| crate::choices::told("Your computer sent a picture that could not be read."))?);
            if part.done {
                break;
            }
            // a part that moves nothing forward and is not the last would loop for ever: what came is only
            // part of the picture, and none of it is kept
            if part.next <= from {
                return Err(crate::choices::told("Your computer sent only part of that picture."));
            }
            from = part.next;
        }
        airband_io::files::write_atomic(&file, &bytes).map_err(|_| RadioError::here("keep that picture"))?;
        Ok(file.display().to_string())
    }
}
