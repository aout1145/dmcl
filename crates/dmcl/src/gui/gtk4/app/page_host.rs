//! 页面宿主：把「实现了 [`Page`] 的组件」挂成标签页，并管它们的生命周期。
//!
//! 页面组件的 `Init` 固定是 [`PageInit`]（标签句柄 + 页面自己的参数）。打开页面用
//! [`PageHost::open_page`]：它启动组件、把根控件 append 成新的标签页、绑好标题句柄、切到新
//! 标签页，并**保活**组件控制器；标签被移除时（`page-detached`）再把它丢掉。

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use relm4::adw;
use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

/// 页面对自己标签标题的句柄：页面随时可以直接改标题。
///
/// `open_page` 是先启动组件、再 `append` 出标签页的，所以页面在 `init` 里拿到句柄时还
/// 没绑定。绑定前调用 [`TabTitle::set_title`] 会先缓存，绑定后补写。
#[derive(Clone, Default)]
pub(crate) struct TabTitle {
    inner: Rc<RefCell<TabTitleInner>>,
}

#[derive(Default)]
struct TabTitleInner {
    /// 绑定后的真实标签页。
    page: Option<adw::TabPage>,
    /// 所属标签视图：`close()` 要用它请求关闭页面。
    view: Option<adw::TabView>,
    /// 绑定前设过、还没写进去的标题。
    pending: Option<String>,
}

impl TabTitle {
    /// 改标签标题。
    pub fn set_title(&self, title: &str) {
        let mut inner = self.inner.borrow_mut();
        match &inner.page {
            Some(page) => page.set_title(title),
            None => inner.pending = Some(title.to_string()),
        }
    }

    /// 请求关闭这个标签页（页面自己可以关闭自己；走 `close-page` 信号，根组件放行）。
    pub fn close(&self) {
        let inner = self.inner.borrow();
        if let (Some(page), Some(view)) = (&inner.page, &inner.view) {
            view.close_page(page);
        }
    }

    /// 绑定真正的标签页与视图，并补写绑定前设置的标题。
    fn bind(&self, page: &adw::TabPage, view: &adw::TabView) {
        let mut inner = self.inner.borrow_mut();
        inner.page = Some(page.clone());
        inner.view = Some(view.clone());
        if let Some(title) = inner.pending.take() {
            page.set_title(&title);
        }
    }
}

/// 能作为标签页打开的页面组件：`Init` 是 [`PageInit`]。
pub(crate) trait Page: Component<Root: IsA<gtk::Widget>> + 'static {
    /// [`PageHost::open_page`] 的实参，也就是 [`PageInit::args`]。
    ///
    /// 参数不止一个时打包成元组（如 `(String, u32)`）；没有参数就是 `()`。
    type Args;
}

/// 页面组件的初始化参数：[`TabTitle`] + 页面自己的参数。
pub(crate) struct PageInit<A> {
    pub tab: TabTitle,
    pub args: A,
}

/// 页面宿主：持有标签视图，管已打开页面的生命周期。
pub(crate) struct PageHost {
    tab_view: adw::TabView,
    /// 保活表：打开的页面（关标签时丢弃）。键是标签页，值是组件控制器。
    opened: RefCell<HashMap<adw::TabPage, Box<dyn Any>>>,
}

impl PageHost {
    pub fn new(tab_view: adw::TabView) -> Self {
        Self {
            tab_view,
            opened: RefCell::new(HashMap::new()),
        }
    }

    /// 打开一个页面，返回它的标签页。
    pub fn open_page<C>(&self, args: C::Args) -> adw::TabPage
    where
        C: Page + Component<Init = PageInit<C::Args>>,
    {
        let tab_title = TabTitle::default();
        let connector = C::builder().launch(PageInit {
            tab: tab_title.clone(),
            args,
        });
        let page = self.tab_view.append(connector.widget());
        tab_title.bind(&page, &self.tab_view);
        // 新开的页面直接切过去，不用等用户再点一下标签。
        self.tab_view.set_selected_page(&page);
        self.opened
            .borrow_mut()
            .insert(page.clone(), Box::new(connector.detach()));
        page
    }

    /// 标签页被移除时丢掉页面的保活（由 `page-detached` 调用）。
    pub fn close(&self, page: &adw::TabPage) {
        self.opened.borrow_mut().remove(page);
    }
}
