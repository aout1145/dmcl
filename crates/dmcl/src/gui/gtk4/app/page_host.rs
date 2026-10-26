//! 页面生命周期：一次性页面用 [`PageHost`] 懒创建，长期保活页面用 [`PageSlot`] 直接持有。
//!
//! 一次性页面（如 `CustomPage`）实现 [`Page`]，`Init` 固定是 [`PageInit`]（标签句柄 + 页面
//! 自己的参数）。打开页面用 [`PageHost::open_page`]：它启动组件、把根控件 append 成新的标签
//! 页、绑好标题句柄、切到新标签页，并**保活**组件控制器；标签被移除时（`page-detached`）再
//! 把它丢掉。
//!
//! 长期保活页面（如主页和设置页）在启动时创建好组件，由 [`PageSlot`] 持有控制器与当前标签；
//! 关闭标签只清空标签句柄，组件本身继续存活，重新打开时把同一个根控件挂到新标签。

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

    /// 请求关闭这个标签页（走 `AdwTabView` 的默认关闭行为；所有页面都允许关闭）。
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

/// 长期保活页面的句柄：组件控制器 + 当前标签。
///
/// 页面在启动时创建，关闭标签只把 [`PageSlot::tab`] 置空，控制器和组件状态继续保留；
/// 再次打开时把同一个根控件挂到新标签。
pub(crate) struct PageSlot<C: Component> {
    pub(crate) controller: Controller<C>,
    /// 当前标签；关闭后为 `None`。
    pub(crate) tab: Option<adw::TabPage>,
}

impl<C> PageSlot<C>
where
    C: Component<Root: IsA<gtk::Widget>>,
{
    /// 已打开就聚焦；否则把组件根控件挂成标签并记录。
    pub fn open(&mut self, view: &adw::TabView, title: &str) {
        match &self.tab {
            Some(tab) => {
                tab.set_title(title);
                view.set_selected_page(tab);
            }
            None => {
                let tab = view.append(self.controller.widget());
                tab.set_title(title);
                view.set_selected_page(&tab);
                self.tab = Some(tab);
            }
        }
    }

    /// 标签被移除时清空句柄。
    pub fn close(&mut self, page: &adw::TabPage) {
        if self.tab.as_ref() == Some(page) {
            self.tab = None;
        }
    }
}
