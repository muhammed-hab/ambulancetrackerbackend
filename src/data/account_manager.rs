mod settings_manager;
mod tracking_manager;

pub use settings_manager::*;
pub use tracking_manager::*;

use serde::{Deserialize, Serialize};
use sqlx::types::Uuid;
use thiserror::Error;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AccountId(pub Uuid);
impl AccountId {
	pub fn new(uuid: Uuid) -> Self {
		Self(uuid)
	}
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SessionToken(pub [u8; 32]);
impl SessionToken {
	pub fn new(bytes: [u8; 32]) -> Self {
		Self(bytes)
	}
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "account_role", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum AccountRole {
	User,
	Admin,
	SiteAdmin
}

impl AccountRole {
	/// Returns whether it is valid for self to own an account of property role.
	///
	/// A SiteAdmin can own an Admin, an Admin can own a user, and a user can own none.
	pub fn can_own(self, property: AccountRole) -> bool {
		match (self, property) {
			(AccountRole::SiteAdmin, AccountRole::Admin) => true,
			(AccountRole::Admin, AccountRole::User) => true,
			_ => false
		}
	}
}

#[derive(Debug, Error)]
pub enum AccountCreationError {
	#[error("A site_admin can only create admins, an admin can only create users, a user cannot create accounts.")]
	InvalidOwnerRole,
	#[error("Specified owner account id not found.")]
	OwnerNotFound,
	#[error("Other error: {0}")]
	Other(Box<dyn std::error::Error>)
}

#[derive(Debug, Error)]
pub enum AccountOwnerManageError {
	#[error("The targeted user is not found, or the account specified as the owner does not own the account for which management is requested.")]
	UserNotFound,
	#[error("Other error: {0}")]
	Other(Box<dyn std::error::Error>)
}

#[derive(Debug, Error)]
pub enum AccountChangePasswordError {
	#[error("The targeted user is not found.")]
	UserNotFound,
	#[error("Incorrect Password")]
	IncorrectPassword,
	#[error("Other error: {0}")]
	Other(Box<dyn std::error::Error>)
}

#[derive(Debug, Error)]
pub enum AccountLoginError {
	#[error("The targeted user is not found.")]
	UserNotFound,
	#[error("Incorrect password")]
	IncorrectPassword,
	#[error("Other error: {0}")]
	Other(Box<dyn std::error::Error>)
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum SessionRetrievalPurpose {
	/// The action for which a session token is necessary is changing a password
	ChangePassword,
	/// The action for which a session token is necessary is not changing a password
	Other
}
#[derive(Debug, Error)]
pub enum SessionRetrievalError {
	#[error("The user must change the password")]
	InvalidPurpose,
	#[error("Session token is not valid or does not exist.")]
	InvalidToken,
	#[error("Other error: {0}")]
	Other(Box<dyn std::error::Error>)
}

#[async_trait::async_trait]
pub trait AccountManager {

	/// Creates a user account which will be owned by the specified admin. Returns the new user id
	/// and a temporary password.
	///
	/// An [AccountRole::SiteAdmin] can only create [AccountRole::Admin] accounts, an
	/// [AccountRole::Admin] can only create [AccountRole::User] accounts, and an [AccountRole::User]
	/// cannot create accounts.
	async fn create_account(&self, owner_id: &AccountId, account_role: AccountRole, username: &str)
		-> Result<(AccountId, String), AccountCreationError>;

	/// Resets the password of an account, returning a new temporary password which must be changed
	/// prior to performing any other action.
	///
	/// The specified owner must be the owner of this account, regardless of the owner role.
	async fn reset_password(&self, owner_id: &AccountId, account_id: &AccountId)
		-> Result<String, AccountOwnerManageError>;

	/// Deletes the specified account and all owned resources.
	///
	/// The specified owner must be the owner of this account, regardless of the owner role.
	async fn delete_account(&self, owner_id: &AccountId, account_id: &AccountId)
		-> Result<(), AccountOwnerManageError>;

	/// Changes a user's password if the provided current password is correct. Note that no password
	/// requirements should be enforced at this level.
	async fn change_password(&self, account_id: &AccountId, current_password: &str, new_password: &str)
		-> Result<(), AccountChangePasswordError>;

	/// Invalidates the provided session token. If the session token does not exist, no action is taken.
	async fn destroy_session(&self, token: &SessionToken)
		-> Result<(), Box<dyn std::error::Error>>;

	/// Attempts to log in the specified user
	async fn login(&self, username: &str, password: &str)
		-> Result<SessionToken, AccountLoginError>;

	/// Attempts to look up a user using the authenticated session token.
	///
	/// If a password reset is necessary, the token is not valid for any purpose but a password reset.
	async fn retrieve_account(&self, session_token: &SessionToken, purpose: SessionRetrievalPurpose)
		-> Result<AccountId, SessionRetrievalError>;
}