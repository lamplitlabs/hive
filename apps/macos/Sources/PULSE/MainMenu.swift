import AppKit

/// The menu bar. Window commands go to the first responder (the key
/// window's ConsoleWindowController); the rest reach the AppDelegate. Keys
/// the console owns are left to it: ⌘K (search), ⌘J (Ask), ⌘↩, Esc, ⌥T.
@MainActor
enum MainMenu {
    /// The sidebar's order (web/hub/src/components/shell/app-shell.tsx).
    static let routes: [(title: String, path: String, key: String)] = [
        ("Home", "/", "1"),
        ("Chat", "/chat/", "2"),
        ("Tasks", "/tasks/", "3"),
        ("Votes", "/votes/", "4"),
        ("Records", "/records/", "5"),
        ("Agents", "/agents/", "6"),
        ("Runs", "/harness/", "7"),
        ("Machines", "/compute/", "8"),
        ("Connections", "/connections/", "9"),
    ]

    static func build() -> NSMenu {
        let main = NSMenu()
        main.addItem(holder(appMenu()))
        main.addItem(holder(fileMenu()))
        main.addItem(holder(editMenu()))
        main.addItem(holder(viewMenu()))
        main.addItem(holder(goMenu()))
        let window = windowMenu()
        main.addItem(holder(window))
        NSApp.windowsMenu = window
        let help = helpMenu()
        main.addItem(holder(help))
        NSApp.helpMenu = help
        return main
    }

    private static func holder(_ menu: NSMenu) -> NSMenuItem {
        let item = NSMenuItem(title: menu.title, action: nil, keyEquivalent: "")
        item.submenu = menu
        return item
    }

    private static func item(
        _ title: String, _ action: Selector?, _ key: String = "", _ modifiers: NSEvent.ModifierFlags = [.command],
        represented: Any? = nil, target: AnyObject? = nil
    ) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: key)
        item.keyEquivalentModifierMask = key.isEmpty ? [] : modifiers
        item.representedObject = represented
        item.target = target
        return item
    }

    private static func appMenu() -> NSMenu {
        let menu = NSMenu(title: "PULSE")
        menu.addItem(item("About PULSE", #selector(AppDelegate.showAbout(_:))))
        menu.addItem(.separator())
        menu.addItem(item("Settings…", #selector(ConsoleWindowController.goToRoute(_:)), ",", represented: "/settings/"))
        menu.addItem(.separator())
        let services = NSMenu(title: "Services")
        let servicesItem = item("Services", nil)
        servicesItem.submenu = services
        NSApp.servicesMenu = services
        menu.addItem(servicesItem)
        menu.addItem(.separator())
        menu.addItem(item("Hide PULSE", #selector(NSApplication.hide(_:)), "h"))
        menu.addItem(item("Hide Others", #selector(NSApplication.hideOtherApplications(_:)), "h", [.command, .option]))
        menu.addItem(item("Show All", #selector(NSApplication.unhideAllApplications(_:))))
        menu.addItem(.separator())
        menu.addItem(item("Quit PULSE", #selector(NSApplication.terminate(_:)), "q"))
        return menu
    }

    private static func fileMenu() -> NSMenu {
        let menu = NSMenu(title: "File")
        menu.addItem(item("New Window", #selector(AppDelegate.newConsoleWindow(_:)), "n"))
        menu.addItem(item("New Tab", #selector(NSResponder.newWindowForTab(_:)), "t"))
        menu.addItem(.separator())
        menu.addItem(item("Open in Browser", #selector(ConsoleWindowController.openInBrowser(_:)), "o", [.command, .shift]))
        menu.addItem(item("Copy Link to This Page", #selector(ConsoleWindowController.copyPageLink(_:)), "c", [.command, .shift]))
        menu.addItem(.separator())
        menu.addItem(item("Close Window", #selector(NSWindow.performClose(_:)), "w"))
        menu.addItem(.separator())
        menu.addItem(item("Print…", #selector(ConsoleWindowController.printConsole(_:)), "p"))
        return menu
    }

    private static func editMenu() -> NSMenu {
        let menu = NSMenu(title: "Edit")
        menu.addItem(item("Undo", Selector(("undo:")), "z"))
        menu.addItem(item("Redo", Selector(("redo:")), "z", [.command, .shift]))
        menu.addItem(.separator())
        menu.addItem(item("Cut", #selector(NSText.cut(_:)), "x"))
        menu.addItem(item("Copy", #selector(NSText.copy(_:)), "c"))
        menu.addItem(item("Paste", #selector(NSText.paste(_:)), "v"))
        menu.addItem(item("Paste and Match Style", #selector(NSTextView.pasteAsPlainText(_:)), "v", [.command, .option, .shift]))
        menu.addItem(item("Delete", #selector(NSText.delete(_:))))
        menu.addItem(item("Select All", #selector(NSText.selectAll(_:)), "a"))
        menu.addItem(.separator())

        let find = NSMenu(title: "Find")
        find.addItem(item("Find…", #selector(ConsoleWindowController.showFind(_:)), "f"))
        find.addItem(item("Find Next", #selector(ConsoleWindowController.findNextMatch(_:)), "g"))
        find.addItem(item("Find Previous", #selector(ConsoleWindowController.findPreviousMatch(_:)), "g", [.command, .shift]))
        find.addItem(item("Use Selection for Find", #selector(ConsoleWindowController.useSelectionForFind(_:)), "e"))
        menu.addItem(submenuItem(find))

        let spelling = NSMenu(title: "Spelling and Grammar")
        spelling.addItem(item("Show Spelling and Grammar", #selector(NSText.showGuessPanel(_:)), ":"))
        spelling.addItem(item("Check Document Now", #selector(NSText.checkSpelling(_:)), ";"))
        spelling.addItem(.separator())
        spelling.addItem(item("Check Spelling While Typing", #selector(NSTextView.toggleContinuousSpellChecking(_:))))
        spelling.addItem(item("Check Grammar With Spelling", #selector(NSTextView.toggleGrammarChecking(_:))))
        spelling.addItem(item("Correct Spelling Automatically", #selector(NSTextView.toggleAutomaticSpellingCorrection(_:))))
        menu.addItem(submenuItem(spelling))

        let substitutions = NSMenu(title: "Substitutions")
        substitutions.addItem(item("Show Substitutions", #selector(NSTextView.orderFrontSubstitutionsPanel(_:))))
        substitutions.addItem(.separator())
        substitutions.addItem(item("Smart Copy/Paste", #selector(NSTextView.toggleSmartInsertDelete(_:))))
        substitutions.addItem(item("Smart Quotes", #selector(NSTextView.toggleAutomaticQuoteSubstitution(_:))))
        substitutions.addItem(item("Smart Dashes", #selector(NSTextView.toggleAutomaticDashSubstitution(_:))))
        substitutions.addItem(item("Smart Links", #selector(NSTextView.toggleAutomaticLinkDetection(_:))))
        substitutions.addItem(item("Text Replacement", #selector(NSTextView.toggleAutomaticTextReplacement(_:))))
        menu.addItem(submenuItem(substitutions))

        let transformations = NSMenu(title: "Transformations")
        transformations.addItem(item("Make Upper Case", #selector(NSResponder.uppercaseWord(_:))))
        transformations.addItem(item("Make Lower Case", #selector(NSResponder.lowercaseWord(_:))))
        transformations.addItem(item("Capitalize", #selector(NSResponder.capitalizeWord(_:))))
        menu.addItem(submenuItem(transformations))

        let speech = NSMenu(title: "Speech")
        speech.addItem(item("Start Speaking", #selector(NSTextView.startSpeaking(_:))))
        speech.addItem(item("Stop Speaking", #selector(NSTextView.stopSpeaking(_:))))
        menu.addItem(submenuItem(speech))
        return menu
    }

    private static func viewMenu() -> NSMenu {
        let menu = NSMenu(title: "View")
        menu.addItem(item("Reload Page", #selector(ConsoleWindowController.reloadConsole(_:)), "r"))
        menu.addItem(item("Reload From PULSE", #selector(ConsoleWindowController.reloadConsoleFromOrigin(_:)), "r", [.command, .shift]))
        menu.addItem(.separator())
        menu.addItem(item("Actual Size", #selector(ConsoleWindowController.actualPageSize(_:)), "0"))
        menu.addItem(item("Zoom In", #selector(ConsoleWindowController.zoomPageIn(_:)), "+"))
        let zoomInEquals = item("Zoom In", #selector(ConsoleWindowController.zoomPageIn(_:)), "=")
        zoomInEquals.isHidden = true
        zoomInEquals.allowsKeyEquivalentWhenHidden = true
        menu.addItem(zoomInEquals)
        menu.addItem(item("Zoom Out", #selector(ConsoleWindowController.zoomPageOut(_:)), "-"))
        menu.addItem(.separator())
        let appearance = NSMenu(title: "Appearance")
        appearance.addItem(item("Light", #selector(ConsoleWindowController.setConsoleAppearance(_:)), represented: "light"))
        appearance.addItem(item("Dark", #selector(ConsoleWindowController.setConsoleAppearance(_:)), represented: "dark"))
        appearance.addItem(item("Match System", #selector(ConsoleWindowController.setConsoleAppearance(_:)), represented: "system"))
        menu.addItem(submenuItem(appearance))
        menu.addItem(.separator())
        menu.addItem(item("Show Browser Panel", #selector(ConsoleWindowController.toggleBeside(_:)), "b", [.command, .shift]))
        menu.addItem(.separator())
        menu.addItem(item("Enter Full Screen", #selector(NSWindow.toggleFullScreen(_:)), "f", [.command, .control]))
        return menu
    }

    private static func goMenu() -> NSMenu {
        let menu = NSMenu(title: "Go")
        menu.addItem(item("Back", #selector(ConsoleWindowController.consoleGoBack(_:)), "["))
        menu.addItem(item("Forward", #selector(ConsoleWindowController.consoleGoForward(_:)), "]"))
        menu.addItem(.separator())
        for route in routes {
            menu.addItem(item(route.title, #selector(ConsoleWindowController.goToRoute(_:)), route.key, represented: route.path))
        }
        menu.addItem(.separator())
        menu.addItem(item("For You", #selector(ConsoleWindowController.showForYou(_:))))
        menu.addItem(item("Code Delivered", #selector(ConsoleWindowController.goToRoute(_:)), represented: "/code-deliveries/"))
        menu.addItem(.separator())
        // The page answers ⌘K and ⌘J itself first (the web view is offered
        // key equivalents before the menu); these items are the way in from
        // the menu bar and when focus is outside the page.
        menu.addItem(item("Search…", #selector(ConsoleWindowController.openSearch(_:)), "k"))
        menu.addItem(item("Ask…", #selector(ConsoleWindowController.openAsk(_:)), "j"))
        return menu
    }

    private static func windowMenu() -> NSMenu {
        let menu = NSMenu(title: "Window")
        menu.addItem(item("Minimize", #selector(NSWindow.performMiniaturize(_:)), "m"))
        menu.addItem(item("Zoom", #selector(NSWindow.performZoom(_:))))
        menu.addItem(.separator())
        menu.addItem(item("Bring All to Front", #selector(NSApplication.arrangeInFront(_:))))
        return menu
    }

    private static func helpMenu() -> NSMenu {
        let menu = NSMenu(title: "Help")
        menu.addItem(item("PULSE Help", #selector(AppDelegate.showPulseHelp(_:))))
        menu.addItem(item("Show Console Log", #selector(AppDelegate.showConsoleLog(_:))))
        return menu
    }

    private static func submenuItem(_ menu: NSMenu) -> NSMenuItem {
        let item = NSMenuItem(title: menu.title, action: nil, keyEquivalent: "")
        item.submenu = menu
        return item
    }
}
