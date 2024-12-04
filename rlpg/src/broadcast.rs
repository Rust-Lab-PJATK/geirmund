use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, RwLock};

#[derive(Debug)]
struct ChannelMessage<T> {
    instance_id: u32,
    item: T,
}

#[derive(Debug)]
pub struct Channel<T> {
    instance_id: u32,
    items: Arc<RwLock<Vec<ChannelMessage<T>>>>,
    markers: Arc<Mutex<HashMap<u32, usize>>>,
}

impl<T> Channel<T>
where
    T: Clone,
{
    pub fn new() -> Self {
        Self {
            instance_id: rand::random(),
            items: Arc::new(RwLock::new(Vec::new())),
            markers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn send(&mut self, item: T) {
        let items = self.items.clone();
        let mut items = items.write().await;

        items.push(ChannelMessage {
            instance_id: self.instance_id,
            item,
        });
    }

    pub async fn recv(&mut self) -> T {
        let items = self.items.clone();
        let items = items.read().await;

        let item = {
            let markers = self.markers.clone();
            let markers = &mut *markers.lock().await;

            let mut instance_marker: usize;

            loop {
                if let Some(v) = markers.get(&self.instance_id) {
                    instance_marker = *v;
                    break;
                }

                let _ = tokio::time::sleep(std::time::Duration::from_millis(10));
            }

            instance_marker += 1;

            let item = &items[instance_marker];

            markers.insert(self.instance_id, instance_marker);

            item.item.clone()
        };

        self.remove_unused_items_if_possible().await;

        item
    }

    async fn remove_unused_items_if_possible(&self) {
        while self.can_remove_top_item().await {
            let items = self.items.clone();
            let mut items = items.write().await;

            items.remove(0);
        }
    }

    async fn can_remove_top_item(&self) -> bool {
        let markers = self.markers.clone();
        let markers = &mut *markers.lock().await;

        markers.iter().all(|e| *e.1 > 0)
    }
}

impl<T> Clone for Channel<T> {
    fn clone(&self) -> Self {
        Self {
            instance_id: rand::random(),
            items: self.items.clone(),
            markers: self.markers.clone(),
        }
    }
}
