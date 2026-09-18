//! The language menu, in the place each platform keeps menus.
//!
//! On macOS that place is the menu bar at the top of the *screen*, which belongs to the
//! application rather than to any window: an app that puts its language choice inside its
//! window has put it somewhere a Mac user does not look. So there we build a real
//! `NSMenu` and hang it off the application's main menu, next to the app menu.
//!
//! Everywhere else there is no menu bar to borrow, so the windows draw their own row and
//! call [`crate::i18n::language_menu`] from it; [`install`] there is a no-op that says so.
//!
//! Nothing here touches the application's own windows — the menu is a separate object
//! owned by the application.

/// Put the language menu in the platform's menu bar.
///
/// `app_name` titles the application menu, which macOS shows in bold. Idempotent: calling
/// it from every frame is harmless (a second call finds the menu already installed and
/// only re-applies the titles).
///
/// Returns true when this platform has a menu bar and the menu is now in it.
#[cfg(target_os = "macos")]
pub fn install(app_name: &str) -> bool {
    native::install(app_name)
}

/// Re-apply the menu's titles and radio marks to the current language.
///
/// Called after a language change, by the window that noticed it.
#[cfg(target_os = "macos")]
pub fn refresh(app_name: &str) {
    native::refresh(app_name);
}

/// No menu bar to put it in: the window draws its own (see [`crate::i18n::language_menu`]).
#[cfg(not(target_os = "macos"))]
pub fn install(_app_name: &str) -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn refresh(_app_name: &str) {}

#[cfg(target_os = "macos")]
mod native {
    use crate::i18n::{self, Lang};
    use objc2::rc::Retained;
    use objc2::runtime::NSObject;
    use objc2::{define_class, msg_send, sel, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSControlStateValueOff, NSControlStateValueOn, NSMenu, NSMenuItem,
    };
    use objc2_foundation::{MainThreadMarker, NSObjectProtocol, NSString};
    use std::cell::RefCell;

    /// The menu, and the items in it that have to be rewritten when the language moves.
    ///
    /// `thread_local` rather than a `static`: none of these objects is `Send`, and none of
    /// them belongs anywhere but the main thread — menus are main-thread-only by AppKit's
    /// own rules, which is the same rule the rest of this file follows.
    struct Installed {
        /// The application menu (the bold one), so its title can follow the language.
        app_item: Option<Retained<NSMenuItem>>,
        /// The "Language" item, whose submenu holds one entry per language.
        menu_item: Retained<NSMenuItem>,
        submenu: Retained<NSMenu>,
        /// One item per [`Lang::ALL`], in that order; the tag is the index into it.
        entries: Vec<Retained<NSMenuItem>>,
        /// Kept alive because `NSMenuItem` holds its target **weakly**: dropping this
        /// would leave the items pointing at freed memory.
        #[allow(dead_code)]
        handler: Retained<Handler>,
    }

    thread_local! {
        static INSTALLED: RefCell<Option<Installed>> = const { RefCell::new(None) };
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and `Handler` has no ivars
        // and implements no Drop.
        #[unsafe(super = NSObject)]
        #[thread_kind = MainThreadOnly]
        #[ivars = ()]
        struct Handler;

        // SAFETY: NSObjectProtocol has no safety requirements.
        unsafe impl NSObjectProtocol for Handler {}

        // The selector matches the method, which takes the menu item that was clicked
        // and returns nothing.
        impl Handler {
            /// Picked from the menu. `tag` is the index into [`Lang::ALL`].
            ///
            /// Nothing else happens here: each window notices the language change on its
            /// next frame and retitles itself, which keeps this object free of any
            /// knowledge about who is running.
            #[unsafe(method(selectLanguage:))]
            fn select_language(&self, sender: &NSMenuItem) {
                let index = sender.tag();
                if let Ok(index) = usize::try_from(index) {
                    if let Some(lang) = Lang::ALL.get(index) {
                        i18n::set_lang(*lang);
                    }
                }
            }
        }
    );

    impl Handler {
        /// The menu's target. objc2 0.6 does not generate a constructor for a class with
        /// no ivars, so this is the two lines the macro's documentation asks for: allocate,
        /// fill in the ivars, then hand it to `NSObject`'s `init`.
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            // SAFETY: `init` on a freshly allocated instance of our own class, which has
            // no ivars to initialise beyond the unit it was given.
            unsafe { msg_send![super(this), init] }
        }
    }

    pub(super) fn install(app_name: &str) -> bool {
        // `MainThreadMarker::new` is None off the main thread; a menu cannot be built there.
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        if INSTALLED.with(|slot| slot.borrow().is_some()) {
            // Already there. The titles are rewritten by `refresh`, which the windows call
            // when they notice a language change — not on every frame.
            return true;
        }

        let app = NSApplication::sharedApplication(mtm);
        let main_menu = match app.mainMenu() {
            Some(menu) => menu,
            None => {
                // No menu bar at all yet: make the smallest one that can hold an app menu
                // and this menu. Quit is the one item whose absence a Mac user notices.
                let menu = NSMenu::new(mtm);
                let app_item = NSMenuItem::new(mtm);
                app_item.setTitle(&NSString::from_str(app_name));
                let app_submenu = NSMenu::new(mtm);
                let quit = NSMenuItem::new(mtm);
                quit.setTitle(&NSString::from_str(&i18n::fill(
                    i18n::t().quit_app,
                    &[&app_name],
                )));
                quit.setKeyEquivalent(&NSString::from_str("q"));
                unsafe {
                    // `terminate:` with no target: the responder chain ends at NSApp.
                    quit.setAction(Some(sel!(terminate:)));
                }
                app_submenu.addItem(&quit);
                app_item.setSubmenu(Some(&app_submenu));
                menu.addItem(&app_item);
                app.setMainMenu(Some(&menu));
                menu
            }
        };

        let handler = Handler::new(mtm);
        let menu_item = NSMenuItem::new(mtm);
        let submenu = NSMenu::new(mtm);

        let mut entries = Vec::new();
        for (index, lang) in Lang::ALL.iter().enumerate() {
            let entry = NSMenuItem::new(mtm);
            entry.setTitle(&NSString::from_str(lang.native_name()));
            entry.setTag(index as isize);
            unsafe {
                // SAFETY: the target outlives the item (it lives in `INSTALLED`) and the
                // selector is the method above, which takes exactly this argument.
                entry.setTarget(Some(&handler));
                entry.setAction(Some(sel!(selectLanguage:)));
            }
            submenu.addItem(&entry);
            entries.push(entry);
        }
        menu_item.setSubmenu(Some(&submenu));
        main_menu.addItem(&menu_item);

        let app_item = main_menu.itemAtIndex(0);
        INSTALLED.with(|slot| {
            *slot.borrow_mut() = Some(Installed {
                app_item,
                menu_item,
                submenu,
                entries,
                handler,
            });
        });
        refresh(app_name);

        // A menu whose action did not stick is a menu that does nothing when clicked; say
        // so rather than shipping a decorative menu.
        let wired = INSTALLED.with(|slot| {
            slot.borrow()
                .as_ref()
                .and_then(|installed| installed.entries.last())
                .and_then(|entry| entry.action())
                .is_some_and(|action| action == sel!(selectLanguage:))
        });
        if !wired {
            eprintln!("sendmecongo: the language menu was built without a working action");
        }
        wired
    }

    pub(super) fn refresh(app_name: &str) {
        let t = i18n::t();
        let live = i18n::current();
        INSTALLED.with(|slot| {
            let slot = slot.borrow();
            let Some(installed) = slot.as_ref() else {
                return;
            };
            if let Some(app_item) = &installed.app_item {
                app_item.setTitle(&NSString::from_str(app_name));
            }
            installed
                .menu_item
                .setTitle(&NSString::from_str(t.language));
            installed.submenu.setTitle(&NSString::from_str(t.language));
            for (index, entry) in installed.entries.iter().enumerate() {
                entry.setState(if Lang::ALL.get(index) == Some(&live) {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
            }
        });
    }
}
