use crate::data::{AccountChangePasswordError, AccountCreationError, AccountId, AccountLoginError, AccountManager, AccountOwnerManageError, AccountRole, SessionRetrievalError, SessionRetrievalPurpose, SessionToken};
use argon2::Argon2;
use rand::TryRngCore;
use sqlx::PgPool;
use std::error::Error;
use std::fmt::{Display, Formatter};

pub struct SqlAccountManager(PgPool);

#[async_trait::async_trait]
impl AccountManager for SqlAccountManager {
	async fn create_account(&self, owner_id: &AccountId, account_role: AccountRole, username: &str) -> Result<(AccountId, String), AccountCreationError> {
		let (owner_role,): (AccountRole,) =
			sqlx::query_as("SELECT role FROM accounts WHERE user_id=$1;")
				.bind(owner_id.0)
				.fetch_optional(&self.0)
				.await
				.map_err(|e| AccountCreationError::Other(e.into()))?
				.ok_or(AccountCreationError::OwnerNotFound)?;
		
		if owner_role.can_own(account_role) {
			self.unchecked_create_account(username, account_role, Some(owner_id)).await.map_err(|e| AccountCreationError::Other(e.into()))
		} else {
			Err(AccountCreationError::InvalidOwnerRole)
		}
	}

	async fn reset_password(&self, owner_id: &AccountId, account_id: &AccountId) -> Result<String, AccountOwnerManageError> {
		let password = random_password(16).map_err(|e| AccountOwnerManageError::Other(e.into()))?;
		let salt = random_salt().map_err(|e| AccountOwnerManageError::Other(e.into()))?;
		let hash = hash_password(password.as_bytes(), &salt).map_err(|e| AccountOwnerManageError::Other(e.into()))?;

		match sqlx::query_as::<_, (i32,)>("UPDATE accounts SET password_salt=$3, password_hash=$4 WHERE user_id=$1 AND owner_id=$2 RETURNING 1;")
			.bind(account_id.0)
			.bind(owner_id.0)
			.bind(salt)
			.bind(hash)
			.fetch_optional(&self.0)
			.await.map_err(|e| AccountOwnerManageError::Other(e.into()))? {
			Some(_) => Ok(password),
			None => Err(AccountOwnerManageError::UserNotFound)
		}
	}

	async fn delete_account(&self, owner_id: &AccountId, account_id: &AccountId) -> Result<(), AccountOwnerManageError> {
		match sqlx::query_as::<_, (i32,)>("DELETE FROM accounts WHERE user_id=$1 AND owner_id=$2 RETURNING 1;")
			.bind(account_id.0)
			.bind(owner_id.0)
			.fetch_optional(&self.0)
			.await.map_err(|e| AccountOwnerManageError::Other(e.into()))? {
			Some(_) => Ok(()),
			None => Err(AccountOwnerManageError::UserNotFound)
		}
	}

	async fn change_password(&self, account_id: &AccountId, current_password: &str, new_password: &str) -> Result<(), AccountChangePasswordError> {
		let (current_hash, current_salt): ([u8; 32], [u8; 16]) =
			sqlx::query_as("SELECT password_hash, password_salt FROM accounts WHERE user_id=$1;")
			.bind(account_id.0)
			.fetch_optional(&self.0)
			.await
			.map_err(|e| AccountChangePasswordError::Other(e.into()))?
			.ok_or(AccountChangePasswordError::UserNotFound)?;

		let check_hash = hash_password(current_password.as_bytes(), &current_salt).map_err(|e| AccountChangePasswordError::Other(e.into()))?;
		if check_hash == current_hash {
			let new_salt = random_salt().map_err(|e| AccountChangePasswordError::Other(e.into()))?;
			let new_hash = hash_password(new_password.as_bytes(), &new_salt).map_err(|e| AccountChangePasswordError::Other(e.into()))?;

			sqlx::query("UPDATE accounts SET password_salt=$2, password_hash=$3, password_reset_needed=false WHERE user_id=$1")
				.bind(account_id.0)
				.bind(new_salt)
				.bind(new_hash)
				.execute(&self.0)
				.await
				.map_err(|e| AccountChangePasswordError::Other(e.into()))?;

			Ok(())
		} else {
			Err(AccountChangePasswordError::IncorrectPassword)
		}
	}

	async fn destroy_session(&self, token: &SessionToken) -> Result<(), Box<dyn Error>> {
		sqlx::query("DELETE FROM sessions WHERE session_id=$1;")
			.bind(token.0)
			.execute(&self.0)
			.await?;
		Ok(())
	}

	async fn login(&self, username: &str, password: &str) -> Result<SessionToken, AccountLoginError> {
		let (hash, salt, user_id): ([u8; 32], [u8; 16], sqlx::types::Uuid) =
			sqlx::query_as("SELECT password_hash, password_salt, user_id FROM accounts WHERE username=$1;")
				.bind(username)
				.fetch_optional(&self.0)
				.await
				.map_err(|e| AccountLoginError::Other(e.into()))?
				.ok_or(AccountLoginError::UserNotFound)?;

		let check_hash = hash_password(password.as_bytes(), &salt)
			.map_err(|e| AccountLoginError::Other(e.into()))?;

		if hash == check_hash {
			let session = random_session().map_err(|e| AccountLoginError::Other(e.into()))?;

			sqlx::query("INSERT INTO sessions (session_id, user_id) VALUES ($1, $2)")
				.bind(session.0)
				.bind(user_id)
				.execute(&self.0)
				.await
				.map_err(|e| AccountLoginError::Other(e.into()))?;
			Ok(session)
		} else {
			Err(AccountLoginError::IncorrectPassword)
		}
	}

	async fn retrieve_account(&self, session_token: &SessionToken, purpose: SessionRetrievalPurpose) -> Result<AccountId, SessionRetrievalError> {
		let (account_id, password_reset_needed): (sqlx::types::Uuid, bool) =
			sqlx::query_as("SELECT accounts.user_id, accounts.password_reset_needed FROM sessions JOIN accounts ON sessions.user_id=accounts.user_id WHERE sessions.session_id=$1;")
			.bind(session_token.0)
			.fetch_optional(&self.0)
			.await
			.map_err(|e| SessionRetrievalError::Other(e.into()))?
			.ok_or(SessionRetrievalError::InvalidToken)?;

		match (purpose, password_reset_needed) {
			(SessionRetrievalPurpose::Other, true) => Err(SessionRetrievalError::InvalidPurpose),
			_ => Ok(AccountId(account_id))
		}
	}
}

/// Creates a random secure password of the specified length.
/// Allowed characters are alphanumeric and `!@#$%^&*()-_=+`
fn random_password(length: usize) -> Result<String, Box<dyn Error>> {
	const ALLOWED_CHARS: [char; 76] = ['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L',
		'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', 'a', 'b', 'c', 'd',
		'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v',
		'w', 'x', 'y', 'z', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '!', '@', '#', '$',
		'%', '^', '&', '*', '(', ')', '-', '_', '=', '+'];

	let mut password = String::with_capacity(length);

	let mut rand = 0u128;

	for _ in 0..length {
		if rand < ALLOWED_CHARS.len() as u128 {
			rand += rand::rngs::OsRng.try_next_u64()? as u128;
		}
		password.push(ALLOWED_CHARS[(rand % ALLOWED_CHARS.len() as u128) as usize]);
	}

	Ok(password)
}

/// Creates a random secure 16 byte salt
fn random_salt() -> Result<[u8; 16], Box<dyn Error>> {
	let mut result = [0u8; 16];
	rand::rngs::OsRng.try_fill_bytes(&mut result)?;
	Ok(result)
}

#[derive(Debug)]
struct HashError(argon2::Error);
impl Error for HashError {}
impl Display for HashError {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.0)
	}
}

/// Creates a 32 byte hash of the specified password and salt
fn hash_password(password: &[u8], salt: &[u8]) -> Result<[u8; 32], HashError> {
	let argon2 = Argon2::default();
	let mut out = [0u8; 32];
	argon2.hash_password_into(password, &salt, &mut out).map_err(|e| HashError(e))?;
	Ok(out)
}

/// Creates a random secure session token
fn random_session() -> Result<SessionToken, Box<dyn Error>> {
	let mut result = [0u8; 32];
	rand::rngs::OsRng.try_fill_bytes(&mut result)?;
	Ok(SessionToken(result))
}

impl SqlAccountManager {
	async fn unchecked_create_account(&self, username: &str, role: AccountRole, owner: Option<&AccountId>) -> Result<(AccountId, String), Box<dyn Error>> {
		let password = random_password(16)?;
		let salt = random_salt()?;
		let hash = hash_password(password.as_bytes(), &salt)?;

		let (account_id, ) = sqlx::query_as("INSERT INTO accounts(username, password_hash, password_salt, role, owner_id) VALUES ($1, $2, $3, $4, $5) RETURNING user_id;")
			.bind(username)
			.bind(hash)
			.bind(salt)
			.bind(role)
			.bind(owner.map(|acc| acc.0))
			.fetch_one(&self.0)
			.await?;

		Ok((AccountId::new(account_id), password))
	}

	/// Creates a new AmbulanceTracker using the specified connection as the backend.
	/// It is expected that the migrations file has been executed already.
	pub fn new(pool: PgPool) -> Self {
		Self(pool)
	}

	pub async fn create_site_admin(&self, username: &str) -> Result<(AccountId, String), Box<dyn Error>> {
		self.unchecked_create_account(username, AccountRole::SiteAdmin, None).await
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use sqlx::PgPool;

	fn mgr(pool: PgPool) -> SqlAccountManager {
		SqlAccountManager::new(pool)
	}

	#[sqlx::test]
	async fn site_admin_can_create_admin(pool: PgPool) {
		let mgr = mgr(pool);

		let (site_admin_id, _) = mgr.unchecked_create_account("root", AccountRole::SiteAdmin, None).await.unwrap();
		let (admin_id, temp_pass) =
			mgr.create_account(&site_admin_id, AccountRole::Admin, "admin1").await.expect("should create admin");

		assert_ne!(admin_id, site_admin_id);
		assert!(!temp_pass.is_empty());
	}

	#[sqlx::test]
	async fn admin_cannot_create_admin(pool: PgPool) {
		let mgr = mgr(pool);

		let (site_admin_id, _) = mgr.unchecked_create_account("root", AccountRole::SiteAdmin, None).await.unwrap();
		let (admin_id, _) =
			mgr.create_account(&site_admin_id, AccountRole::Admin, "a1").await.unwrap();

		let result =
			mgr.create_account(&admin_id, AccountRole::Admin, "illegal").await;

		assert!(matches!(result, Err(AccountCreationError::InvalidOwnerRole)));
	}

	#[sqlx::test]
	async fn admin_can_create_user(pool: PgPool) {
		let mgr = mgr(pool);

		let (site_admin_id, _) = mgr.unchecked_create_account("root", AccountRole::SiteAdmin, None).await.unwrap();
		let (admin_id, _) =
			mgr.create_account(&site_admin_id, AccountRole::Admin, "a1").await.unwrap();

		let (user_id, temp_pass) =
			mgr.create_account(&admin_id, AccountRole::User, "user1").await.unwrap();

		assert_ne!(user_id, admin_id);
		assert!(!temp_pass.is_empty());
	}

	#[sqlx::test]
	async fn user_cannot_create_any_account(pool: PgPool) {
		let mgr = mgr(pool);

		let (site_admin_id, _) = mgr.unchecked_create_account("root", AccountRole::SiteAdmin, None).await.unwrap();
		let (admin_id, _) =
			mgr.create_account(&site_admin_id, AccountRole::Admin, "a1").await.unwrap();
		let (user_id, _) =
			mgr.create_account(&admin_id, AccountRole::User, "u1").await.unwrap();

		let result =
			mgr.create_account(&user_id, AccountRole::User, "u2").await;

		assert!(matches!(result, Err(AccountCreationError::InvalidOwnerRole)));
	}

	#[sqlx::test]
	async fn password_reset_requires_correct_owner(pool: PgPool) {
		let mgr = mgr(pool);

		let (site_admin_id, _) = mgr.unchecked_create_account("root", AccountRole::SiteAdmin, None).await.unwrap();
		let (admin_id, _) =
			mgr.create_account(&site_admin_id, AccountRole::Admin, "a1").await.unwrap();
		let (user_id, _) =
			mgr.create_account(&admin_id, AccountRole::User, "u1").await.unwrap();

		// Wrong owner
		let wrong_result =
			mgr.reset_password(&user_id, &user_id).await;

		assert!(matches!(wrong_result, Err(AccountOwnerManageError::UserNotFound)));

		// Correct owner
		let new_pw =
			mgr.reset_password(&admin_id, &user_id).await.expect("admin should reset user password");

		assert!(!new_pw.is_empty());
	}

	#[sqlx::test]
	async fn delete_account_removes_user_and_resources(pool: PgPool) {
		let mgr = mgr(pool);

		let (site_admin_id, _) = mgr.unchecked_create_account("root", AccountRole::SiteAdmin, None).await.unwrap();
		let (admin_id, _) =
			mgr.create_account(&site_admin_id, AccountRole::Admin, "a1").await.unwrap();
		let (user_id, _) =
			mgr.create_account(&admin_id, AccountRole::User, "u1").await.unwrap();

		// Deleting with wrong owner must fail
		let wrong_res = mgr.delete_account(&user_id, &admin_id).await;
		assert!(matches!(wrong_res, Err(AccountOwnerManageError::UserNotFound)));

		// Delete with correct owner
		mgr.delete_account(&admin_id, &user_id).await.expect("admin should delete user");

		// Ensure user can no longer log in
		let login_res = mgr.login("u1", "anything").await;
		assert!(matches!(login_res, Err(AccountLoginError::UserNotFound)));
	}

	#[sqlx::test]
	async fn login_requires_correct_password(pool: PgPool) {
		let mgr = mgr(pool);

		let (site_admin_id, _) = mgr.unchecked_create_account("root", AccountRole::SiteAdmin, None).await.unwrap();
		let (_, temp_pass) =
			mgr.create_account(&site_admin_id, AccountRole::Admin, "a1").await.unwrap();

		// Wrong password
		let wrong = mgr.login("a1", "badpw").await;
		assert!(matches!(wrong, Err(AccountLoginError::IncorrectPassword)));

		// Correct
		let token = mgr.login("a1", &temp_pass).await.expect("valid login");
		assert_eq!(token.0.len(), 32);
	}

	#[sqlx::test]
	async fn destroy_session_invalidates_token(pool: PgPool) {
		let mgr = mgr(pool);

		let (site_admin_id, _) = mgr.unchecked_create_account("root", AccountRole::SiteAdmin, None).await.unwrap();
		let (_, temp_pass) =
			mgr.create_account(&site_admin_id, AccountRole::Admin, "a1").await.unwrap();

		let token =
			mgr.login("a1", &temp_pass).await.expect("should log in");

		// Destroy it
		mgr.destroy_session(&token).await.expect("destroy should succeed");

		// Retrieval should now fail
		let res =
			mgr.retrieve_account(&token, SessionRetrievalPurpose::Other).await;

		assert!(matches!(res, Err(SessionRetrievalError::InvalidToken)));
	}

	#[sqlx::test]
	async fn session_retrieval_requires_valid_token(pool: PgPool) {
		let mgr = mgr(pool);

		let (site_admin_id, _) = mgr.unchecked_create_account("root", AccountRole::SiteAdmin, None).await.unwrap();
		let (admin_id, temp_pass) =
			mgr.create_account(&site_admin_id, AccountRole::Admin, "a1").await.unwrap();

		let token =
			mgr.login("a1", &temp_pass).await.expect("login succeeds");

		let retrieved =
			mgr.retrieve_account(&token, SessionRetrievalPurpose::ChangePassword)
				.await
				.expect("session retrieval must succeed");
		assert_eq!(retrieved, admin_id, "retrieve_account should return correct account");

		assert!(matches!(mgr.retrieve_account(&token, SessionRetrievalPurpose::Other).await, Err(SessionRetrievalError::InvalidPurpose)));

		mgr.change_password(&admin_id, &temp_pass, &temp_pass).await.unwrap();

		let retrieved =
			mgr.retrieve_account(&token, SessionRetrievalPurpose::Other)
				.await
				.expect("session retrieval must succeed");
		assert_eq!(retrieved, admin_id, "retrieve_account should return correct account");

		let retrieved =
			mgr.retrieve_account(&token, SessionRetrievalPurpose::ChangePassword)
				.await
				.expect("session retrieval must succeed");
		assert_eq!(retrieved, admin_id, "retrieve_account should return correct account");
	}
}

