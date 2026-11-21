use anyhow::{Context, Result, bail};
use pnet_datalink::{DataLinkReceiver, DataLinkSender};
use std::time::{Duration, Instant};

pub struct Link {
    pub source: [u8; 6],
    sender: Box<dyn DataLinkSender>,
    receiver: Box<dyn DataLinkReceiver>,
}

impl Link {
    pub fn new(
        source: [u8; 6],
        sender: Box<dyn DataLinkSender>,
        receiver: Box<dyn DataLinkReceiver>,
    ) -> Self {
        Self {
            source,
            sender,
            receiver,
        }
    }

    pub fn send(&mut self, frame: &[u8]) -> Result<()> {
        match self.sender.send_to(frame, None) {
            Some(Ok(())) => Ok(()),
            Some(Err(error)) => Err(error).context("failed to transmit Ethernet frame"),
            None => bail!("the selected interface rejected the Ethernet frame"),
        }
    }

    /// Send a request and collect matching HomePlug responses until timeout.
    pub fn request(
        &mut self,
        frame: &[u8],
        ethertype: u16,
        expected_mmes: &[u16],
        expected_source: Option<[u8; 6]>,
        timeout: Duration,
    ) -> Result<Vec<Vec<u8>>> {
        self.send(frame)?;

        let deadline = Instant::now() + timeout;
        let mut responses = Vec::new();
        while Instant::now() < deadline {
            match self.receiver.next() {
                Ok(packet) => {
                    if packet.len() < 17
                        || u16::from_be_bytes([packet[12], packet[13]]) != ethertype
                        || !expected_mmes.contains(&u16::from_le_bytes([packet[15], packet[16]]))
                    {
                        continue;
                    }
                    let source: [u8; 6] = packet[6..12].try_into().expect("six-byte MAC");
                    if expected_source.is_some_and(|mac| mac != source) {
                        continue;
                    }
                    responses.push(packet.to_vec());
                    if expected_source.is_some() {
                        break;
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) => {}
                Err(error) => return Err(error).context("failed reading Ethernet frames"),
            }
        }
        Ok(responses)
    }
}
