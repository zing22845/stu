use std::sync::{mpsc, Arc};
use tokio::spawn;

use crate::{
    client::Client,
    config::Config,
    error::{AppError, Result},
    event::{
        AppEventType, CompleteDownloadObjectResult, CompleteInitializeResult,
        CompleteLoadObjectResult, CompleteLoadObjectsResult,
    },
    file::{save_binary, save_error_log},
    item::{AppObjects, FileDetail, FileVersion, Item},
};

pub struct AppListState {
    pub selected: usize,
    pub offset: usize,
    height: usize,
}

impl AppListState {
    fn new(height: usize) -> AppListState {
        AppListState {
            selected: 0,
            offset: 0,
            height: AppListState::calc_list_height(height),
        }
    }

    fn select_next(&mut self) {
        if self.selected - self.offset == self.height - 1 {
            self.offset += 1;
        }
        self.selected += 1;
    }

    fn select_prev(&mut self) {
        if self.selected - self.offset == 0 {
            self.offset -= 1;
        }
        self.selected -= 1;
    }

    fn select_next_page(&mut self, total: usize) {
        if total < self.height {
            self.selected = total - 1;
            self.offset = 0;
        } else if self.selected + self.height < total - 1 {
            self.selected += self.height;
            if self.selected + self.height > total - 1 {
                self.offset = total - self.height;
            } else {
                self.offset = self.selected;
            }
        } else {
            self.selected = total - 1;
            self.offset = total - self.height;
        }
    }

    fn select_prev_page(&mut self, total: usize) {
        if total < self.height {
            self.selected = 0;
            self.offset = 0;
        } else if self.selected > self.height {
            self.selected -= self.height;
            if self.selected < self.height {
                self.offset = 0;
            } else {
                self.offset = self.selected - self.height + 1;
            }
        } else {
            self.selected = 0;
            self.offset = 0;
        }
    }

    fn select_first(&mut self) {
        self.selected = 0;
        self.offset = 0;
    }

    fn select_last(&mut self, total: usize) {
        self.selected = total - 1;
        if self.height < total {
            self.offset = total - self.height;
        }
    }

    fn calc_list_height(height: usize) -> usize {
        height - 3 /* header */ - 2 /* footer */ - 2 /* list area border */
    }
}

#[derive(PartialEq, Eq, Clone)]
pub enum ViewState {
    Initializing,
    BucketList,
    ObjectList,
    Detail(DetailViewState),
    Help(Box<ViewState>),
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum DetailViewState {
    Detail = 0,
    Version = 1,
}

pub enum Notification {
    None,
    Info(String),
    Error(String),
}

pub struct AppViewState {
    pub list_state: AppListState,
    pub view_state: ViewState,
    pub notification: Notification,
    pub is_loading: bool,
}

impl AppViewState {
    fn new(height: usize) -> AppViewState {
        AppViewState {
            list_state: AppListState::new(height),
            view_state: ViewState::Initializing,
            notification: Notification::None,
            is_loading: true,
        }
    }
}

pub struct App {
    pub app_view_state: AppViewState,
    app_objects: AppObjects,
    current_keys: Vec<String>,
    client: Option<Arc<Client>>,
    config: Option<Config>,
    tx: mpsc::Sender<AppEventType>,
}

impl App {
    pub fn new(tx: mpsc::Sender<AppEventType>, height: usize) -> App {
        App {
            app_view_state: AppViewState::new(height),
            app_objects: AppObjects::new(),
            current_keys: Vec::new(),
            client: None,
            config: None,
            tx,
        }
    }

    pub fn initialize(&mut self, config: Config, client: Client) {
        self.config = Some(config);
        self.client = Some(Arc::new(client));

        let client = self.client.as_ref().unwrap().clone();
        let tx = self.tx.clone();
        spawn(async move {
            let buckets = client.load_all_buckets().await;
            let result = CompleteInitializeResult::new(buckets);
            tx.send(AppEventType::CompleteInitialize(result)).unwrap();
        });
    }

    pub fn complete_initialize(&mut self, result: Result<CompleteInitializeResult>) {
        match result {
            Ok(CompleteInitializeResult { buckets }) => {
                self.app_objects.set_items(Vec::new(), buckets);
                self.app_view_state.view_state = ViewState::ObjectList;
            }
            Err(e) => {
                self.tx.send(AppEventType::Error(e)).unwrap();
            }
        }
        self.app_view_state.is_loading = false;
    }

    pub fn resize(&mut self, height: usize) {
        self.app_view_state.list_state.height = AppListState::calc_list_height(height);
        // todo: adjust
    }

    pub fn current_key_string(&self) -> String {
        format!(" {} ", self.current_keys.join(" / "))
    }

    fn current_bucket(&self) -> String {
        self.current_keys[0].clone()
    }

    fn current_bucket_opt(&self) -> Option<&String> {
        self.current_keys.get(0)
    }

    fn current_object_prefix(&self) -> String {
        let mut prefix = String::new();
        for key in &self.current_keys[1..] {
            prefix.push_str(key);
            prefix.push('/');
        }
        prefix
    }

    pub fn current_items(&self) -> Vec<Item> {
        self.app_objects.get_items(&self.current_keys)
    }

    fn current_items_len(&self) -> usize {
        self.app_objects.get_items_len(&self.current_keys)
    }

    fn get_current_selected(&self) -> Option<&Item> {
        let i = self.app_view_state.list_state.selected;
        self.app_objects.get_item(&self.current_keys, i)
    }

    pub fn get_current_file_detail(&self) -> Option<&FileDetail> {
        self.get_current_selected().and_then(|selected| {
            if let Item::File { name, .. } = selected {
                let bucket = &self.current_bucket();
                let prefix = &self.current_object_prefix();
                let key = &self.object_detail_map_key(bucket, prefix, name);
                self.app_objects.get_object_detail(key)
            } else {
                None
            }
        })
    }

    pub fn get_current_file_versions(&self) -> Option<&Vec<FileVersion>> {
        self.get_current_selected().and_then(|selected| {
            if let Item::File { name, .. } = selected {
                let bucket = &self.current_bucket();
                let prefix = &self.current_object_prefix();
                let key = &self.object_detail_map_key(bucket, prefix, name);
                self.app_objects.get_object_versions(key)
            } else {
                None
            }
        })
    }

    pub fn select_next(&mut self) {
        match self.app_view_state.view_state {
            ViewState::Initializing | ViewState::Detail(_) | ViewState::Help(_) => {}
            ViewState::BucketList | ViewState::ObjectList => {
                let current_selected = self.app_view_state.list_state.selected;
                let len = self.current_items_len();
                if len == 0 || current_selected >= len - 1 {
                    self.app_view_state.list_state.select_first();
                } else {
                    self.app_view_state.list_state.select_next();
                };
            }
        }
    }

    pub fn select_prev(&mut self) {
        match self.app_view_state.view_state {
            ViewState::Initializing | ViewState::Detail(_) | ViewState::Help(_) => {}
            ViewState::BucketList | ViewState::ObjectList => {
                let current_selected = self.app_view_state.list_state.selected;
                let len = self.current_items_len();
                if len == 0 {
                    self.app_view_state.list_state.select_first();
                } else if current_selected == 0 {
                    self.app_view_state.list_state.select_last(len);
                } else {
                    self.app_view_state.list_state.select_prev();
                };
            }
        }
    }

    pub fn select_next_page(&mut self) {
        match self.app_view_state.view_state {
            ViewState::Initializing | ViewState::Detail(_) | ViewState::Help(_) => {}
            ViewState::BucketList | ViewState::ObjectList => {
                let len = self.current_items_len();
                self.app_view_state.list_state.select_next_page(len)
            }
        }
    }

    pub fn select_prev_page(&mut self) {
        match self.app_view_state.view_state {
            ViewState::Initializing | ViewState::Detail(_) | ViewState::Help(_) => {}
            ViewState::BucketList | ViewState::ObjectList => {
                let len = self.current_items_len();
                self.app_view_state.list_state.select_prev_page(len)
            }
        }
    }

    pub fn select_first(&mut self) {
        match self.app_view_state.view_state {
            ViewState::Initializing | ViewState::Detail(_) | ViewState::Help(_) => {}
            ViewState::BucketList | ViewState::ObjectList => {
                self.app_view_state.list_state.select_first();
            }
        }
    }

    pub fn select_last(&mut self) {
        match self.app_view_state.view_state {
            ViewState::Initializing | ViewState::Detail(_) | ViewState::Help(_) => {}
            ViewState::BucketList | ViewState::ObjectList => {
                let len = self.current_items_len();
                self.app_view_state.list_state.select_last(len);
            }
        }
    }

    pub fn move_down(&mut self) {
        match self.app_view_state.view_state {
            ViewState::Initializing | ViewState::Detail(_) | ViewState::Help(_) => {}
            ViewState::BucketList | ViewState::ObjectList => {
                if let Some(selected) = self.get_current_selected() {
                    if let Item::File { .. } = selected {
                        if self.exists_current_object_detail() {
                            self.app_view_state.view_state =
                                ViewState::Detail(DetailViewState::Detail);
                        } else {
                            self.tx.send(AppEventType::LoadObject).unwrap();
                            self.app_view_state.is_loading = true;
                        }
                    } else {
                        self.current_keys.push(selected.name().to_owned());
                        self.app_view_state.list_state.select_first();

                        if !self.exists_current_objects() {
                            self.tx.send(AppEventType::LoadObjects).unwrap();
                            self.app_view_state.is_loading = true;
                        }
                    }
                }
            }
        }
    }

    fn exists_current_object_detail(&self) -> bool {
        let bucket = &self.current_bucket();
        let prefix = &self.current_object_prefix();
        match self.get_current_selected() {
            Some(selected) => {
                let map_key = &self.object_detail_map_key(bucket, prefix, selected.name());
                self.app_objects.exists_object_details(map_key)
            }
            None => false,
        }
    }

    fn exists_current_objects(&self) -> bool {
        self.app_objects.exists_item(&self.current_keys)
    }

    pub fn move_up(&mut self) {
        match self.app_view_state.view_state {
            ViewState::Initializing => {}
            ViewState::BucketList | ViewState::ObjectList => {
                let key = self.current_keys.pop();
                if key.is_some() {
                    self.app_view_state.list_state.select_first();
                }
            }
            ViewState::Detail(_) => {
                self.app_view_state.view_state = ViewState::ObjectList;
            }
            ViewState::Help(_) => {
                self.toggle_help();
            }
        }
    }

    pub fn back_to_bucket_list(&mut self) {
        match self.app_view_state.view_state {
            ViewState::Initializing | ViewState::Detail(_) | ViewState::Help(_) => {}
            ViewState::BucketList | ViewState::ObjectList => {
                self.current_keys.clear();
            }
        }
    }

    pub fn load_objects(&self) {
        let bucket = self.current_bucket();
        let prefix = self.current_object_prefix();
        let client = self.client.as_ref().unwrap().clone();
        let tx = self.tx.clone();
        spawn(async move {
            let items = client.load_objects(&bucket, &prefix).await;
            let result = CompleteLoadObjectsResult::new(items);
            tx.send(AppEventType::CompleteLoadObjects(result)).unwrap();
        });
    }

    pub fn complete_load_objects(&mut self, result: Result<CompleteLoadObjectsResult>) {
        match result {
            Ok(CompleteLoadObjectsResult { items }) => {
                self.app_objects.set_items(self.current_keys.clone(), items);
            }
            Err(e) => {
                self.tx.send(AppEventType::Error(e)).unwrap();
            }
        }
        self.app_view_state.is_loading = false;
    }

    pub fn load_object(&self) {
        if let Some(Item::File {
            name, size_byte, ..
        }) = self.get_current_selected()
        {
            let name = name.clone();
            let size_byte = *size_byte;

            let bucket = self.current_bucket();
            let prefix = self.current_object_prefix();
            let key = format!("{}{}", prefix, name);

            let map_key = self.object_detail_map_key(&bucket, &prefix, &name);

            let client = self.client.as_ref().unwrap().clone();
            let tx = self.tx.clone();

            spawn(async move {
                let detail = client
                    .load_object_detail(&bucket, &key, &name, size_byte)
                    .await;
                let versions = client.load_object_versions(&bucket, &key).await;
                let result = CompleteLoadObjectResult::new(detail, versions, map_key);
                tx.send(AppEventType::CompleteLoadObject(result)).unwrap();
            });
        }
    }

    pub fn complete_load_object(&mut self, result: Result<CompleteLoadObjectResult>) {
        match result {
            Ok(CompleteLoadObjectResult {
                detail,
                versions,
                map_key,
            }) => {
                self.app_objects
                    .set_object_details(&map_key, detail, versions);
                self.app_view_state.view_state = ViewState::Detail(DetailViewState::Detail);
            }
            Err(e) => {
                self.tx.send(AppEventType::Error(e)).unwrap();
            }
        }
        self.app_view_state.is_loading = false;
    }

    fn object_detail_map_key(&self, bucket: &String, prefix: &String, name: &String) -> String {
        format!("{}/{}{}", bucket, prefix, name)
    }

    pub fn select_tabs(&mut self) {
        match self.app_view_state.view_state {
            ViewState::Initializing
            | ViewState::BucketList
            | ViewState::ObjectList
            | ViewState::Help(_) => {}
            ViewState::Detail(vs) => match vs {
                DetailViewState::Detail => {
                    self.app_view_state.view_state = ViewState::Detail(DetailViewState::Version);
                }
                DetailViewState::Version => {
                    self.app_view_state.view_state = ViewState::Detail(DetailViewState::Detail);
                }
            },
        }
    }

    pub fn toggle_help(&mut self) {
        match &self.app_view_state.view_state {
            ViewState::Initializing => {}
            ViewState::Help(before) => {
                self.app_view_state.view_state = *before.clone();
            }
            ViewState::BucketList | ViewState::ObjectList | ViewState::Detail(_) => {
                let before = self.app_view_state.view_state.clone();
                self.app_view_state.view_state = ViewState::Help(Box::new(before));
            }
        }
    }

    pub fn download(&mut self) {
        match self.app_view_state.view_state {
            ViewState::Initializing
            | ViewState::BucketList
            | ViewState::ObjectList
            | ViewState::Help(_) => {}
            ViewState::Detail(_) => {
                self.tx.send(AppEventType::DownloadObject).unwrap();
                self.app_view_state.is_loading = true;
            }
        }
    }

    pub fn download_object(&self) {
        if let Some(Item::File { name, .. }) = self.get_current_selected() {
            let bucket = self.current_bucket();
            let prefix = self.current_object_prefix();
            let key = format!("{}{}", prefix, name);

            let config = self.config.as_ref().unwrap();
            let path = config.download_file_path(name);

            let client = self.client.as_ref().unwrap().clone();
            let tx = self.tx.clone();

            spawn(async move {
                let bytes = client.download_object(&bucket, &key).await;
                let result = CompleteDownloadObjectResult::new(bytes, path);
                tx.send(AppEventType::CompleteDownloadObject(result))
                    .unwrap();
            });
        }
    }

    pub fn complete_download_object(&mut self, result: Result<CompleteDownloadObjectResult>) {
        let result = match result {
            Ok(CompleteDownloadObjectResult { path, bytes }) => {
                save_binary(&path, &bytes).map(|_| path)
            }
            Err(e) => Err(e),
        };
        match result {
            Ok(path) => {
                let msg = format!("Download completed successfully: {}", path);
                self.tx.send(AppEventType::Info(msg)).unwrap();
            }
            Err(e) => {
                self.tx.send(AppEventType::Error(e)).unwrap();
            }
        }
        self.app_view_state.is_loading = false;
    }

    pub fn save_error(&self, e: &AppError) {
        let config = self.config.as_ref().unwrap();
        // cause panic if save errors
        let path = config.error_log_path().unwrap();
        save_error_log(&path, e).unwrap();
    }

    pub fn open_management_console(&self) {
        let client = self.client.as_ref().unwrap();
        let bucket = self.current_bucket_opt();

        let result = match self.app_view_state.view_state {
            ViewState::Initializing | ViewState::Help(_) => Ok(()),
            ViewState::BucketList | ViewState::ObjectList => match bucket {
                Some(bucket) => {
                    let prefix = self.current_object_prefix();
                    client.open_management_console_list(bucket, &prefix)
                }
                None => client.open_management_console_buckets(),
            },
            ViewState::Detail(_) => {
                if let Some(Item::File { name, .. }) = self.get_current_selected() {
                    let prefix = self.current_object_prefix();
                    client.open_management_console_object(bucket.unwrap(), &prefix, name)
                } else {
                    Err(AppError::msg("Failed to get current selected item"))
                }
            }
        };
        if let Err(e) = result {
            self.tx.send(AppEventType::Error(e)).unwrap();
        }
    }
}
