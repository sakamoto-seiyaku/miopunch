//! Shared management commands and privilege classification for all platforms.
use p2panda_auth::{
    Access,
    group::{GroupAction, GroupMember},
};
use p2panda_core::VerifyingKey;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagementAction {
    AddMember(VerifyingKey),
    RemoveMember(VerifyingKey),
    AppointAdministrator(VerifyingKey),
    DemoteAdministrator(VerifyingKey),
}

impl ManagementAction {
    pub(crate) fn target(self) -> VerifyingKey {
        match self {
            Self::AddMember(target)
            | Self::RemoveMember(target)
            | Self::AppointAdministrator(target)
            | Self::DemoteAdministrator(target) => target,
        }
    }
}

impl ManagementAction {
    pub(crate) fn requires_genesis(self, current: Option<Access>) -> bool {
        matches!(
            self,
            Self::AppointAdministrator(_) | Self::DemoteAdministrator(_)
        ) || matches!(self, Self::RemoveMember(_)) && current == Some(Access::manage())
    }
    pub(crate) fn group_action(self, current: Option<Access>) -> GroupAction<VerifyingKey, ()> {
        let member = GroupMember::Individual(self.target());
        match self {
            Self::AddMember(_) => GroupAction::Add {
                member,
                access: Access::read(),
            },
            Self::RemoveMember(_) => GroupAction::Remove { member },
            Self::AppointAdministrator(_) if current.is_some() => GroupAction::Promote {
                member,
                access: Access::manage(),
            },
            Self::AppointAdministrator(_) => GroupAction::Add {
                member,
                access: Access::manage(),
            },
            Self::DemoteAdministrator(_) => GroupAction::Demote {
                member,
                access: Access::read(),
            },
        }
    }
}
