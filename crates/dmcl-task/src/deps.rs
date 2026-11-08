//! 依赖、任务句柄，以及类型擦除后的依赖边。

use std::any::Any;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::event::{TaskRef, TaskStatus};
use crate::graph::StepId;
use crate::runtime::Runtime;

/// 一个静态任务节点的引用。
///
/// 静态句柄**没有** `wait`，只能作为下游节点的 deps 被注入：
///
/// ```compile_fail
/// fn needs_wait<O: Send + Sync + 'static>(handle: dmcl_task::Handle<O>) {
///     let _ = handle.wait();
/// }
/// ```
pub type Handle<O> = Arc<dyn TaskHandle<O>>;

/// 一个动态任务节点的引用；多一个 [`WaitableTaskHandle::wait`]。
///
/// 动态句柄**不是** [`Deps`]，不能写进别的节点的 deps：
///
/// ```compile_fail
/// fn requires_deps<D: dmcl_task::Deps>(_: D) {}
///
/// fn bad<O: Send + Sync + 'static>(handle: dmcl_task::DynamicHandle<O>) {
///     requires_deps(handle);
/// }
/// ```
pub type DynamicHandle<O> = Arc<dyn WaitableTaskHandle<O>>;

pub(crate) mod sealed {
    use super::*;

    /// 只用在本 crate 内部的句柄能力：外部既不能命名，也不能实现它。
    pub trait SealedHandle<O>: Send + Sync {
        fn dependency(&self) -> Dependency;
        fn resolved(&self) -> Arc<O>;
    }
}

/// 静态任务句柄：只能作为下游节点的依赖被注入取值。
pub trait TaskHandle<O: Send + Sync + 'static>:
    sealed::SealedHandle<O> + Send + Sync + 'static
{
    /// 事件 / UI 使用的任务引用。
    fn task_ref(&self) -> TaskRef;
}

/// 动态任务句柄：在 [`TaskHandle`] 之上增加「等待 + 强制取值」。
#[async_trait::async_trait]
pub trait WaitableTaskHandle<O: Send + Sync + 'static>: TaskHandle<O> {
    /// 等待该节点及其全部动态子节点结束，成功后返回返回值。
    async fn wait(&self) -> Result<Arc<O>>;
}

/// 类型擦除后的依赖边，只用于调度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Dependency {
    pub(crate) graph: u64,
    pub(crate) node: usize,
}

/// 一个节点在句柄与调度器之间共享的槽位。
pub(crate) struct NodeSlot {
    pub(crate) graph: u64,
    pub(crate) id: usize,
    pub(crate) step: StepId,
    pub(crate) name: Arc<str>,
    status: AtomicU8,
    output: Mutex<Option<Arc<dyn Any + Send + Sync>>>,
}

impl NodeSlot {
    pub(crate) fn new(graph: u64, id: usize, step: StepId, name: Arc<str>) -> Arc<Self> {
        Arc::new(Self {
            graph,
            id,
            step,
            name,
            status: AtomicU8::new(TaskStatus::Waiting.as_u8()),
            output: Mutex::new(None),
        })
    }

    pub(crate) fn status(&self) -> TaskStatus {
        TaskStatus::from_u8(self.status.load(Ordering::SeqCst))
    }

    pub(crate) fn set_status(&self, status: TaskStatus) {
        self.status.store(status.as_u8(), Ordering::SeqCst);
    }

    pub(crate) fn set_output(&self, output: Arc<dyn Any + Send + Sync>) {
        let mut slot = self
            .output
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *slot = Some(output);
    }

    pub(crate) fn output_any(&self) -> Option<Arc<dyn Any + Send + Sync>> {
        self.output
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub(crate) fn resolved<O: Send + Sync + 'static>(&self) -> Arc<O> {
        let slot = self
            .output
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        slot.as_ref()
            .expect("节点在依赖完成前被取值")
            .clone()
            .downcast::<O>()
            .expect("任务节点的输出类型没有对齐")
    }

    pub(crate) fn task_ref(&self) -> TaskRef {
        TaskRef::new(self.id, self.step, self.name.clone(), self.status())
    }
}

pub(crate) struct StaticHandle<O> {
    pub(crate) slot: Arc<NodeSlot>,
    marker: PhantomData<fn() -> O>,
}

impl<O: Send + Sync + 'static> StaticHandle<O> {
    pub(crate) fn new(slot: Arc<NodeSlot>) -> Self {
        Self {
            slot,
            marker: PhantomData,
        }
    }
}

impl<O: Send + Sync + 'static> sealed::SealedHandle<O> for StaticHandle<O> {
    fn dependency(&self) -> Dependency {
        Dependency {
            graph: self.slot.graph,
            node: self.slot.id,
        }
    }

    fn resolved(&self) -> Arc<O> {
        self.slot.resolved()
    }
}

impl<O: Send + Sync + 'static> TaskHandle<O> for StaticHandle<O> {
    fn task_ref(&self) -> TaskRef {
        self.slot.task_ref()
    }
}

pub(crate) struct DynamicHandleImpl<O> {
    pub(crate) slot: Arc<NodeSlot>,
    runtime: Arc<Runtime>,
    marker: PhantomData<fn() -> O>,
}

impl<O: Send + Sync + 'static> DynamicHandleImpl<O> {
    pub(crate) fn new(slot: Arc<NodeSlot>, runtime: Arc<Runtime>) -> Self {
        Self {
            slot,
            runtime,
            marker: PhantomData,
        }
    }
}

impl<O: Send + Sync + 'static> sealed::SealedHandle<O> for DynamicHandleImpl<O> {
    fn dependency(&self) -> Dependency {
        Dependency {
            graph: self.slot.graph,
            node: self.slot.id,
        }
    }

    fn resolved(&self) -> Arc<O> {
        self.slot.resolved()
    }
}

impl<O: Send + Sync + 'static> TaskHandle<O> for DynamicHandleImpl<O> {
    fn task_ref(&self) -> TaskRef {
        self.slot.task_ref()
    }
}

#[async_trait::async_trait]
impl<O: Send + Sync + 'static> WaitableTaskHandle<O> for DynamicHandleImpl<O> {
    async fn wait(&self) -> Result<Arc<O>> {
        self.runtime.wait_for_node(self.slot.id).await?;
        Ok(self.slot.resolved())
    }
}

/// 节点需要的依赖。
///
/// * 没有依赖：`()`，`Output = ()`；
/// * 一个依赖：`Handle<T>`，`Output = Arc<T>`；
/// * 数量不定的同类型依赖：`Vec<Handle<T>>`，`Output = Vec<Arc<T>>`；
/// * 多个依赖：`(D1, D2, …)`，每个元素本身都是 `Deps`，`Output = (D1::Output, D2::Output, …)`。
///
/// tuple 里的元素可以是 [`Handle`]，也可以是 [`Vec<Handle<T>>`]，因此「若干固定依赖 +
/// 一组动态数量的依赖」可以直接写在同一个 tuple 里。
///
/// 动态句柄 [`DynamicHandle`] 不实现本 trait，因此不能写进 deps。
pub trait Deps: Send + Sync + 'static {
    type Output: Send + 'static;

    /// 给调度器用：这些依赖全部完成后节点才能启动。
    fn dependencies(&self) -> Vec<Dependency>;

    /// 给任务用：依赖已经完成，取出它们的返回值。
    fn resolve(&self) -> Self::Output;
}

impl Deps for () {
    type Output = ();

    fn dependencies(&self) -> Vec<Dependency> {
        Vec::new()
    }

    fn resolve(&self) {}
}

impl<O: Send + Sync + 'static> Deps for Handle<O> {
    type Output = Arc<O>;

    fn dependencies(&self) -> Vec<Dependency> {
        vec![sealed::SealedHandle::dependency(self.as_ref())]
    }

    fn resolve(&self) -> Self::Output {
        sealed::SealedHandle::resolved(self.as_ref())
    }
}

impl<O: Send + Sync + 'static> Deps for Vec<Handle<O>> {
    type Output = Vec<Arc<O>>;

    fn dependencies(&self) -> Vec<Dependency> {
        self.iter().flat_map(Deps::dependencies).collect()
    }

    fn resolve(&self) -> Self::Output {
        self.iter().map(Deps::resolve).collect()
    }
}

macro_rules! impl_deps_tuple {
    ($(($index:tt, $type:ident)),+) => {
        impl<$($type: Deps),+> Deps for ($($type,)+) {
            type Output = ($($type::Output,)+);

            fn dependencies(&self) -> Vec<Dependency> {
                let mut dependencies = Vec::new();
                $(dependencies.extend(self.$index.dependencies());)+
                dependencies
            }

            fn resolve(&self) -> Self::Output {
                ($(self.$index.resolve(),)+)
            }
        }
    };
}

impl_deps_tuple!((0, A), (1, B));
impl_deps_tuple!((0, A), (1, B), (2, C));
impl_deps_tuple!((0, A), (1, B), (2, C), (3, D));
impl_deps_tuple!((0, A), (1, B), (2, C), (3, D), (4, E));
impl_deps_tuple!((0, A), (1, B), (2, C), (3, D), (4, E), (5, F));
impl_deps_tuple!((0, A), (1, B), (2, C), (3, D), (4, E), (5, F), (6, G));
impl_deps_tuple!(
    (0, A),
    (1, B),
    (2, C),
    (3, D),
    (4, E),
    (5, F),
    (6, G),
    (7, H)
);
impl_deps_tuple!(
    (0, A),
    (1, B),
    (2, C),
    (3, D),
    (4, E),
    (5, F),
    (6, G),
    (7, H),
    (8, I)
);
impl_deps_tuple!(
    (0, A),
    (1, B),
    (2, C),
    (3, D),
    (4, E),
    (5, F),
    (6, G),
    (7, H),
    (8, I),
    (9, J)
);
impl_deps_tuple!(
    (0, A),
    (1, B),
    (2, C),
    (3, D),
    (4, E),
    (5, F),
    (6, G),
    (7, H),
    (8, I),
    (9, J),
    (10, K)
);
impl_deps_tuple!(
    (0, A),
    (1, B),
    (2, C),
    (3, D),
    (4, E),
    (5, F),
    (6, G),
    (7, H),
    (8, I),
    (9, J),
    (10, K),
    (11, L)
);
