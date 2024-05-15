use tracing::{debug, warn};

use crate::models::DurationInSeconds;
use crate::utils::now;
use crate::TimestampInSeconds;
use ockam_core::compat::collections::HashMap;
use ockam_core::compat::sync::Arc;
use ockam_core::compat::uuid::Uuid;
use ockam_core::compat::vec::Vec;
use ockam_core::errcode::{Kind, Origin};
use ockam_core::Error;
use ockam_core::{Result, Route};
use ockam_node::compat::asynchronous::RwLock;

/// If more that MAX_PAYLOAD_PART_UPDATE has elapsed between an update and the one before
/// We consider that the message will never be completed and we drop all the parts
const MAX_PAYLOAD_PART_UPDATE: DurationInSeconds = DurationInSeconds(60);

/// The PayloadCollector stores payload parts that can be possibly received out of order.
///
/// A secure channel message payload can indeed be divided in several parts when it is too large.
/// Each part contains:
///
///   - A UUID identifying the overall payload
///   - The onward route for the message
///   - The return route for the message
///   - The number of the current part
///   - The total number of parts
///
/// When a part is received:
///
///  - It is checks for consistency with other parts of the same message (same onward_route, same return_route, same total number of parts)
///  - If the part has already been received, a warning is emitted but no error is raised
///  - If that part is the last one that was expected for this message, the full payload is reconstituted
///
pub struct PayloadCollector {
    parts: Arc<RwLock<HashMap<Uuid, PayloadParts>>>,
}

impl PayloadCollector {
    /// Create a new PayloadCollector
    pub fn new() -> PayloadCollector {
        PayloadCollector {
            parts: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Update the tracked payloads with a new payload part
    ///  - If the new part completes the full payload then the payload is assembled and returned
    ///  - If the part is the first one for a given payload UUID (and there are more expected parts)
    ///    then a new PayloadParts struct is created to track all the parts for that message payload
    pub async fn update(
        &self,
        payload_uuid: Uuid,
        onward_route: &Route,
        return_route: &Route,
        payload: Vec<u8>,
        current_part_number: u32,
        total_number_of_parts: u32,
    ) -> Result<Option<Vec<u8>>> {
        debug!(onward_route = %onward_route, return_route = %return_route, "receiving a new payload part for message {payload_uuid}: {current_part_number}/{total_number_of_parts}");

        let mut all_parts = self.parts.write().await;
        // We temporarily remove the list of tracked parts for the UUID.
        // We add it back later if the list is still incomplete.
        // There might be a better way where all the updates are done in place.
        let result = match all_parts.remove(&payload_uuid) {
            Some(mut payload_parts) => {
                payload_parts.validate(
                    onward_route,
                    return_route,
                    current_part_number,
                    total_number_of_parts,
                )?;
                if payload_parts.is_complete_with(current_part_number) {
                    let message = payload_parts.complete(current_part_number, payload)?;
                    all_parts.remove(&payload_uuid);
                    debug!("The payload for message {payload_uuid} is now complete");
                    Ok(Some(message))
                } else {
                    let parts_number = payload_parts.update(current_part_number, payload)?;
                    all_parts.insert(payload_uuid, payload_parts);
                    debug!("The payload for message {payload_uuid} is not yet complete, received {} parts/{total_number_of_parts}", parts_number);
                    Ok(None)
                }
            }
            None => {
                if current_part_number > total_number_of_parts {
                    return Err(Error::new(Origin::Channel, Kind::Invalid, format!("Incorrect part number for {payload_uuid}. It is '{current_part_number}', while the total number of expected parts is '{total_number_of_parts}'")));
                };
                // check, just in case, if only one part is expected for the whole payload
                if total_number_of_parts == 1 {
                    warn!("One message has been received and the payload is complete. However a multipart payload should have at least 2 parts");
                    Ok(Some(payload))
                } else {
                    let mut payloads_parts = PayloadParts::new(
                        &payload_uuid,
                        onward_route,
                        return_route,
                        total_number_of_parts,
                    )?;
                    payloads_parts.update(current_part_number, payload)?;
                    all_parts.insert(payload_uuid, payloads_parts);
                    debug!("Storing the first part of the payload for message {payload_uuid}: {current_part_number}/{total_number_of_parts}");
                    Ok(None)
                }
            }
        };

        // Keep only the payload parts that have been recently updated
        let now = now()?;
        all_parts.retain(|_, parts| parts.last_update.add(MAX_PAYLOAD_PART_UPDATE) >= now);
        result
    }

    /// Return the current number of payloads being tracked
    #[cfg(test)]
    async fn payloads_number(&self) -> Result<usize> {
        let parts = self.parts.read().await;
        Ok(parts.len())
    }
}

/// List of received payload parts for a given message payload
struct PayloadParts {
    uuid: Uuid,
    parts: HashMap<u32, Vec<u8>>,
    onward_route: Route,
    return_route: Route,
    expected_total_number_of_parts: u32,
    last_update: TimestampInSeconds,
}

impl PayloadParts {
    /// Create a new list of payload parts, starting from the first received payload part
    fn new(
        uuid: &Uuid,
        onward_route: &Route,
        return_route: &Route,
        expected_total_number: u32,
    ) -> Result<PayloadParts> {
        Ok(PayloadParts {
            uuid: *uuid,
            parts: HashMap::new(),
            onward_route: onward_route.clone(),
            return_route: return_route.clone(),
            expected_total_number_of_parts: expected_total_number,
            last_update: now()?,
        })
    }

    /// Validate a newly received payload part, to make sure that its data is consistent with the
    /// previously received parts
    fn validate(
        &self,
        onward_route: &Route,
        return_route: &Route,
        current_part_number: u32,
        total_number_of_parts: u32,
    ) -> Result<()> {
        // check that the part routes and the other parts routes are the same
        if &self.onward_route != onward_route {
            return Err(Error::new(Origin::Channel, Kind::Conflict, format!("Incorrect onward route for part {current_part_number}/{total_number_of_parts} of message {}. Expected: {}, Got: {}", &self.uuid, &self.onward_route, onward_route)));
        };
        if &self.return_route != return_route {
            return Err(Error::new(Origin::Channel, Kind::Conflict, format!("Incorrect return route for part {current_part_number}/{total_number_of_parts} of message {}. Expected: {}, Got: {}", &self.uuid, &self.return_route, return_route)));
        };
        if self.expected_total_number_of_parts != total_number_of_parts {
            return Err(Error::new(Origin::Channel, Kind::Conflict, format!("Incorrect total number of parts for part {current_part_number} of message {}. Expected: {}, Got: {total_number_of_parts}", &self.uuid, self.expected_total_number_of_parts)));
        };
        if self.expected_total_number_of_parts < current_part_number {
            return Err(Error::new(Origin::Channel, Kind::Conflict, format!("Incorrect part number for part {current_part_number} of message {}. It should less or equal than {total_number_of_parts}", &self.uuid)));
        };
        if self.parts.contains_key(&current_part_number) {
            warn!(
                "The part {current_part_number} has already been received for message {}",
                &self.uuid
            );
            return Ok(());
        };
        Ok(())
    }

    /// Accept the new part and add it with the other parts
    /// Return the current number of parts
    fn update(&mut self, current_payload_number: u32, payload: Vec<u8>) -> Result<usize> {
        self.parts.insert(current_payload_number, payload);
        self.last_update = now()?;
        Ok(self.parts.len())
    }

    /// Check the current payload part would make the full payload complete
    fn is_complete_with(&self, current_payload_number: u32) -> bool {
        !self.parts.contains_key(&current_payload_number)
            && self.parts.len() as u32 == self.expected_total_number_of_parts - 1
    }

    /// Use the current payload part and all the previously received ones to reconstitute the whole
    /// payload
    fn complete(self, current_payload_number: u32, payload: Vec<u8>) -> Result<Vec<u8>> {
        let mut values: Vec<(u32, Vec<u8>)> = self.parts.into_iter().collect();
        values.push((current_payload_number, payload));
        values.sort_by_key(|kv| kv.0);
        let mut result: Vec<u8> = vec![];
        for (_, mut v) in values.into_iter() {
            result.append(&mut v);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ockam_core::route;
    use ockam_core::Result;
    use uuid::uuid;

    #[test]
    fn test_validate_update_and_complete() {
        let uuid = uuid!("02f09a3f-1624-3b1d-8409-44eff7708208");

        // In this test we expect to receive 3 parts. The received order is 2, 3, 1

        // Receiving the first part
        let mut payload_parts =
            PayloadParts::new(&uuid, &route!["onward"], &route!["return"], 3).unwrap();
        assert!(
            payload_parts
                .validate(&route!["onward"], &route!["return"], 2, 3)
                .is_ok(),
            "the first part is validated"
        );
        // update the list
        payload_parts
            .update(2, "second".as_bytes().to_vec())
            .unwrap();

        // validate other parts that would be incorrect
        assert!(
            payload_parts
                .validate(&route!["onward_x"], &route!["return"], 3, 3)
                .is_err(),
            "the onward route must be correct"
        );
        assert!(
            payload_parts
                .validate(&route!["onward"], &route!["return_x"], 3, 3)
                .is_err(),
            "the return route must be correct"
        );
        assert!(
            payload_parts
                .validate(&route!["onward"], &route!["return"], 3, 4)
                .is_err(),
            "the total number of parts must be correct"
        );

        // receive the 3rd part
        assert!(
            payload_parts
                .validate(&route!["onward"], &route!["return"], 3, 3)
                .is_ok(),
            "the third part is validated"
        );
        assert!(!payload_parts.is_complete_with(3));
        let parts_number = payload_parts
            .update(3, "third".as_bytes().to_vec())
            .unwrap();
        assert_eq!(parts_number, 2);

        // receive the 1st part to complete the payload
        assert!(
            payload_parts
                .validate(&route!["onward"], &route!["return"], 1, 3)
                .is_ok(),
            "the first part is validated"
        );
        assert!(payload_parts.is_complete_with(1));
        let payload = payload_parts
            .complete(1, "first".as_bytes().to_vec())
            .unwrap();
        assert_eq!(payload, "firstsecondthird".as_bytes().to_vec());
    }

    #[tokio::test]
    async fn test_payload_collector() -> Result<()> {
        let collector = PayloadCollector::new();

        let uuid1 = uuid!("02f09a3f-1624-3b1d-8409-44eff7708201");
        let uuid2 = uuid!("02f09a3f-1624-3b1d-8409-44eff7708202");
        let uuid3 = uuid!("02f09a3f-1624-3b1d-8409-44eff7708203");
        let uuid4 = uuid!("02f09a3f-1624-3b1d-8409-44eff7708204");

        let result = collector
            .update(
                uuid1,
                &route!["onward1"],
                &route!["return1"],
                "1-1/3,".as_bytes().to_vec(),
                1,
                3,
            )
            .await?;
        assert_eq!(result, None);
        assert_eq!(collector.payloads_number().await?, 1);

        let result = collector
            .update(
                uuid2,
                &route!["onward2"],
                &route!["return2"],
                "2-2/2,".as_bytes().to_vec(),
                2,
                2,
            )
            .await?;
        assert_eq!(result, None);
        assert_eq!(
            collector.payloads_number().await?,
            2,
            "two payloads are now tracked"
        );

        // The last part for message 2 has been received
        let result = collector
            .update(
                uuid2,
                &route!["onward2"],
                &route!["return2"],
                "2-1/2,".as_bytes().to_vec(),
                1,
                2,
            )
            .await?;
        assert_eq!(
            result,
            Some("2-1/2,2-2/2,".as_bytes().to_vec()),
            "parts are returned in order"
        );
        assert_eq!(collector.payloads_number().await?, 1);

        let result = collector
            .update(
                uuid1,
                &route!["onward1"],
                &route!["return1"],
                "1-3/3,".as_bytes().to_vec(),
                3,
                3,
            )
            .await?;
        assert_eq!(result, None);
        assert_eq!(collector.payloads_number().await?, 1);

        // This is the last message for payload 1
        let result = collector
            .update(
                uuid1,
                &route!["onward1"],
                &route!["return1"],
                "1-2/3,".as_bytes().to_vec(),
                2,
                3,
            )
            .await?;
        assert_eq!(
            result,
            Some("1-1/3,1-2/3,1-3/3,".as_bytes().to_vec()),
            "parts are returned in order"
        );
        assert_eq!(collector.payloads_number().await?, 0);

        // A message with just one part in total must also be accepted
        let result = collector
            .update(
                uuid3,
                &route!["onward3"],
                &route!["return3"],
                "3-1/1,".as_bytes().to_vec(),
                1,
                1,
            )
            .await?;
        assert_eq!(
            result,
            Some("3-1/1,".as_bytes().to_vec()),
            "parts are returned in order"
        );
        assert_eq!(collector.payloads_number().await?, 0);

        // A message with an inconsistent part number must be rejected
        let result = collector
            .update(
                uuid4,
                &route!["x"],
                &route!["x"],
                "x".as_bytes().to_vec(),
                2,
                1,
            )
            .await;
        assert!(result.is_err());
        assert_eq!(collector.payloads_number().await?, 0);

        Ok(())
    }
}
