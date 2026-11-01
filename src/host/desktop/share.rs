//! `Host::share_files` for the native Apple platforms.
//!
//! * iOS: the bytes go to `<tmp>/immagine-share-<n>/<name>` (so the receiver sees the real file name) and a
//!   `UIActivityViewController` is presented from the topmost view controller of the foreground window scene. On iPad
//!   it is anchored as a popover in the middle of the screen. The future resolves from `completionWithItemsHandler`.
//! * macOS: same temp file, shown with an `NSSharingServicePicker` at the bottom left of the key window. It resolves
//!   when the delegate reports the choice of a service (or none); the chosen service keeps running after that, so the
//!   temp file stays until the next share (older `immagine-share-*` folders are removed first).
//! * Any other platform: `Err`.
//!
//! A cancelled sheet is an `Err("AbortError: ...")`, which the viewer already treats as "nothing to report".
//! Host calls run on the main thread, which UIKit/AppKit need; off the main thread this returns an `Err`.

#[cfg(any(target_os = "ios", target_os = "macos"))]
mod apple {
    use crate::host::ShareFile;
    use futures_channel::oneshot;
    use std::path::PathBuf;

    pub const CANCELLED: &str = "AbortError: Teilen abgebrochen";

    /// Writes the files to a fresh folder below the temp dir and removes the folders of earlier shares. Returns the
    /// folder and the file paths.
    fn temp_files(files: &[ShareFile]) -> Result<(PathBuf, Vec<PathBuf>), String> {
        let root = std::env::temp_dir();
        if let Ok(entries) = std::fs::read_dir(&root) {
            for e in entries.flatten() {
                if e.file_name().to_string_lossy().starts_with("immagine-share-") {
                    let _ = std::fs::remove_dir_all(e.path());
                }
            }
        }
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = root.join(format!("immagine-share-{nanos}"));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut paths = Vec::with_capacity(files.len());
        for f in files {
            let name: String = f.name.chars().map(|c| if c == '/' || c == '\\' || c == ':' { '_' } else { c }).collect();
            let name = if name.is_empty() || name.starts_with('.') { format!("datei{name}") } else { name };
            let path = dir.join(name);
            std::fs::write(&path, &f.bytes).map_err(|e| e.to_string())?;
            paths.push(path);
        }
        Ok((dir, paths))
    }

    #[cfg(target_os = "ios")]
    pub async fn share(files: Vec<ShareFile>) -> Result<(), String> {
        use block2::RcBlock;
        use objc2::rc::Retained;
        use objc2::runtime::AnyObject;
        use objc2::runtime::Bool;
        use objc2::MainThreadMarker;
        use objc2_foundation::{NSPoint as CGPoint, NSRect as CGRect, NSSize as CGSize, NSArray, NSError, NSString, NSURL};
        use objc2_ui_kit::*;
        use std::cell::RefCell;

        let mtm = MainThreadMarker::new().ok_or("Teilen: nicht im Hauptthread")?;
        let (dir, paths) = temp_files(&files)?;
        drop(files);
        let (tx, rx) = oneshot::channel::<bool>();
        let started: Result<(), String> = (|| {
            let urls: Vec<Retained<AnyObject>> = paths
                .iter()
                .map(|path| unsafe { Retained::cast_unchecked(NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))) })
                .collect();
            let items: Retained<NSArray<AnyObject>> = NSArray::from_retained_slice(&urls);
            let avc = unsafe { UIActivityViewController::initWithActivityItems_applicationActivities(mtm.alloc(), &items, None) };

            // Topmost view controller of the foreground active window scene.
            let app = UIApplication::sharedApplication(mtm);
            let scenes = unsafe { app.connectedScenes() };
            let mut chosen = None;
            for scene in scenes.iter() {
                let Ok(ws) = scene.downcast::<UIWindowScene>() else { continue };
                if ws.activationState() == UISceneActivationState::ForegroundActive {
                    chosen = Some(ws);
                    break;
                }
                chosen.get_or_insert(ws);
            }
            let ws = chosen.ok_or("Teilen: kein Fenster")?;
            let windows = ws.windows();
            let window = windows.iter().find(|w| w.isKeyWindow()).or_else(|| windows.firstObject()).ok_or("Teilen: kein Fenster")?;
            let mut top = window.rootViewController().ok_or("Teilen: kein Fenster")?;
            while let Some(next) = top.presentedViewController() {
                top = next;
            }

            // iPad: the sheet is a popover and crashes without an anchor.
            if let Some(pop) = unsafe { avc.popoverPresentationController() } {
                if let Some(view) = top.view() {
                    let b = view.bounds();
                    let rect = CGRect::new(CGPoint::new(b.origin.x + b.size.width / 2., b.origin.y + b.size.height / 2.), CGSize::new(0., 0.));
                    unsafe {
                        pop.setSourceView(Some(&view));
                        pop.setSourceRect(rect);
                        pop.setPermittedArrowDirections(UIPopoverArrowDirection::empty());
                    }
                }
            }

            let tx = RefCell::new(Some(tx));
            let handler = RcBlock::new(move |_t: *mut NSString, completed: Bool, _items: *mut NSArray<AnyObject>, _err: *mut NSError| {
                if let Some(tx) = tx.borrow_mut().take() {
                    let _ = tx.send(completed.as_bool());
                }
            });
            unsafe { avc.setCompletionWithItemsHandler(&*handler as *const _ as *mut _) };
            unsafe { top.presentViewController_animated_completion(&avc, true, None) };
            Ok(())
        })();
        if let Err(e) = started {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
        let completed = rx.await.unwrap_or(false);
        let _ = std::fs::remove_dir_all(&dir);
        if completed { Ok(()) } else { Err(CANCELLED.into()) }
    }

    #[cfg(target_os = "macos")]
    pub async fn share(files: Vec<ShareFile>) -> Result<(), String> {
        use objc2::rc::Retained;
        use objc2::runtime::{AnyObject, ProtocolObject};
        use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
        use objc2_app_kit::*;
        use objc2_foundation::{NSArray, NSObject, NSObjectProtocol, NSPoint, NSRect, NSRectEdge, NSSize, NSString, NSURL};
        use std::cell::RefCell;

        struct Ivars {
            tx: RefCell<Option<oneshot::Sender<bool>>>,
        }
        define_class!(
            #[unsafe(super(NSObject))]
            #[thread_kind = MainThreadOnly]
            #[name = "ImmagineShareDelegate"]
            #[ivars = Ivars]
            struct ShareDelegate;

            unsafe impl NSObjectProtocol for ShareDelegate {}

            unsafe impl NSSharingServicePickerDelegate for ShareDelegate {
                // Called with `None` when the picker is dismissed without a choice.
                #[unsafe(method(sharingServicePicker:didChooseSharingService:))]
                fn did_choose(&self, _picker: &NSSharingServicePicker, service: Option<&NSSharingService>) {
                    if let Some(tx) = self.ivars().tx.borrow_mut().take() {
                        let _ = tx.send(service.is_some());
                    }
                }
            }
        );

        let mtm = MainThreadMarker::new().ok_or("Teilen: nicht im Hauptthread")?;
        let (dir, paths) = temp_files(&files)?;
        drop(files);
        let (tx, rx) = oneshot::channel::<bool>();
        let keep = (|| {
            let urls: Vec<Retained<AnyObject>> = paths
                .iter()
                .map(|path| unsafe { Retained::cast_unchecked(NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))) })
                .collect();
            let items: Retained<NSArray<AnyObject>> = NSArray::from_retained_slice(&urls);
            let picker = unsafe { NSSharingServicePicker::initWithItems(mtm.alloc(), &items) };
            let delegate: Retained<ShareDelegate> = {
                let this = mtm.alloc::<ShareDelegate>().set_ivars(Ivars { tx: RefCell::new(Some(tx)) });
                unsafe { msg_send![super(this), init] }
            };
            picker.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            let app = NSApplication::sharedApplication(mtm);
            // The picker dismisses itself at once when its app is not active (e.g. started from a background process).
            #[allow(deprecated)]
            app.activateIgnoringOtherApps(true);
            let view = app.keyWindow().or_else(|| app.mainWindow()).and_then(|w| w.contentView()).ok_or("Teilen: kein Fenster")?;
            let rect = NSRect::new(NSPoint::new(24., 24.), NSSize::new(1., 1.));
            picker.showRelativeToRect_ofView_preferredEdge(rect, &view, NSRectEdge::MaxY);
            Ok::<_, String>((picker, delegate))
        })();
        let keep = match keep {
            Ok(k) => k,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                return Err(e);
            }
        };
        let chosen = rx.await.unwrap_or(false);
        drop(keep);
        if chosen {
            Ok(())
        } else {
            let _ = std::fs::remove_dir_all(&dir);
            Err(CANCELLED.into())
        }
    }
}

#[cfg(any(target_os = "ios", target_os = "macos"))]
pub use apple::share;

#[cfg(not(any(target_os = "ios", target_os = "macos")))]
pub async fn share(_files: Vec<crate::host::ShareFile>) -> Result<(), String> {
    Err("Teilen wird auf dieser Plattform nicht unterstützt".into())
}
