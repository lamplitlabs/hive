import Foundation

/// The script behind the menu's "Decisions Only You Can Make": it scrolls to
/// the hard-stop strip on Sessions without a reload when the strip is on
/// screen, and answers false otherwise, so the app loads `/chat/#hard-stops`
/// instead (ConsoleWindowController.showForYou). It used to answer true on
/// any Sessions page, so with a conversation open - where Sessions renders
/// no strip - the menu item did nothing (TASK-149 item 18).
public enum HardStopsScript {
    /// Where the app goes when the script answers false.
    public static let path = "/chat/#hard-stops"

    public static let source = """
    (() => {
      if (location.pathname !== '/chat/') return false;
      const section = document.getElementById('hard-stops');
      if (!section) return false;
      section.scrollIntoView({ behavior: 'smooth', block: 'start' });
      history.replaceState(history.state, '', '#hard-stops');
      return true;
    })()
    """
}
