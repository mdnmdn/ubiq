use super::*;

impl AppState {
    /// Which of the Control screen's two pages is up.
    ///
    /// It starts the refresh loop as well as the tab strip does, so a window restored straight
    /// into Control — where no `set_rail_mode` is ever called — still asks for a reading.
    pub fn set_stats_tab(&mut self, tab: StatsTab, cx: &mut Context<Self>) {
        self.stats.tab = tab;
        self.poll_stats(cx);
        cx.notify();
    }

    /// Ask for the numbers while the Control screen is up, and stop the moment it is not.
    ///
    /// The one place in the interface that polls. Everything else is push, because everything else
    /// changes when something happens; memory and uptime change when nothing does.
    pub(super) fn poll_stats(&mut self, cx: &mut Context<Self>) {
        if self.stats.polling || self.workbench.rail_mode != RailMode::Control {
            return;
        }
        self.stats.polling = true;
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            loop {
                // Three ways out, and all of them are here rather than in the body: the window has
                // gone, the screen has gone, or the mode changed while the timer was running.
                let asked = this.update(cx, |this, _| {
                    if this.workbench.rail_mode != RailMode::Control {
                        this.stats.polling = false;
                        return false;
                    }
                    this.bus.send(Message::ListStats);
                    true
                });
                if !matches!(asked, Ok(true)) {
                    break;
                }
                cx.background_executor().timer(Duration::from_secs(2)).await;
            }
        })
        .detach();
    }
}
