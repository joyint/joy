// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! The token a job carries (JI-01A7-0F).
//!
//! The job of an AI member runs on a platform, and the platform owns
//! nothing that grants authority. What the AI may do in a job comes from
//! the person who put the job to work: when they open it, and every time
//! they hand it back to its AI, joy issues a delegation token for the
//! job's assignee in their name: valid seven days, for this one job,
//! with their grant as it stands. It is sealed for the platform the job
//! runs on and stored at the job, with the platform, the expiry and the
//! opener in clear.
//!
//! The platform opens the seal with its own key and redeems a session
//! per round ([`session`]); with it the AI takes the job and records its
//! rounds as itself, under its delegator, and the guard keeps that
//! session to the job ([`crate::guard`]). A token that expired, was not
//! sealed by the person it names, or no longer matches their grant or
//! delegation is no token: the job goes back to new with the reason.

use chrono::{DateTime, Duration, Utc};

use super::redeem::{redeem_ai_session, RedeemedSession};
use super::{platform, session, token, PublicKey};
use crate::identity::Identity;
use crate::migrations::ai_member_name;
use crate::model::item::{Item, JobFeedback, JobToken, Status};
use crate::model::project::{MemberKind, Project};

pub use joy_token::job::{JobTokenRequest, SealedJobToken, DAYS};

/// What binds the seal to this job of this project.
fn info(project_id: &str, job_id: &str) -> Vec<u8> {
    joy_token::job::seal_info(project_id, job_id)
}

/// The AI member a job is assigned to, by its key in the project.
fn ai_assignee(project: &Project, job: &Item) -> Option<String> {
    let assignee = job.assignees.first()?.member.id();
    project.is_ai(assignee).then(|| assignee.to_string())
}

/// Whether `by` hands the job to its AI assignee with the step that
/// brought it from `from` to where it stands now, so that the step
/// needs a fresh token: a person approves it (new -> open), sets it to
/// in-progress themselves, or answers the AI's question.
pub fn handed_over(project: &Project, job: &Item, from: &Status, by: &Identity) -> bool {
    if ai_assignee(project, job).is_none()
        || project.kind_of(by.member.id()) != MemberKind::Person
        || by.delegated_by.is_some()
    {
        return false;
    }
    let answered = job.job.as_ref().and_then(|spec| spec.feedback) == Some(JobFeedback::Received);
    match (&job.status, from) {
        (Status::Open, Status::New) => true,
        (Status::InProgress, from) if *from != Status::InProgress => true,
        (Status::Review | Status::InProgress, _) => answered,
        _ => false,
    }
}

/// What the token of `job` says when the person `opener_key` issues it
/// now for the platform `domain`: read off the project, for whichever
/// host then issues it with their seed ([`joy_token::job::issue`]).
///
/// The person needs a delegation to the job's assignee; issuing a job's
/// token never makes one.
pub fn request(
    project: &Project,
    job: &Item,
    opener_key: &str,
    domain: &str,
) -> Result<JobTokenRequest, String> {
    platform::fits(project)?;
    let assignee = ai_assignee(project, job)
        .ok_or_else(|| format!("job {} is not assigned to an AI member", job.id))?;
    let name = ai_member_name::read(&assignee).to_string();
    let platform_key = platform::key_of(project, domain)?;
    let opener = project
        .member_by_key(opener_key)
        .ok_or_else(|| format!("{opener_key} is not a member of this project"))?;
    let entry = opener.delegation_to(&assignee).ok_or_else(|| {
        format!("you have not delegated to {name} yet: joy auth token add {name}")
    })?;
    let delegation_salt = entry.delegation_salt.clone().ok_or_else(|| {
        format!("your delegation to {name} is from an older joy: joy auth delegation rotate {name}")
    })?;
    Ok(JobTokenRequest {
        ai_member: assignee.clone(),
        opener: opener_key.to_string(),
        project_id: session::project_id_of(project),
        delegation_salt,
        delegation_verifier: entry.delegation_verifier.clone(),
        grant: super::grants::token_grant(opener, &assignee),
        job: job.id.clone(),
        platform: domain.to_string(),
        platform_key: platform_key.to_hex(),
    })
}

/// A sealed token as the job carries it.
pub fn carried(request: &JobTokenRequest, sealed: SealedJobToken) -> JobToken {
    JobToken {
        platform: request.platform.clone(),
        expires: sealed.expires,
        opener: request.opener.clone().into(),
        sealed: sealed.sealed,
    }
}

/// Issue the token a [`request`] describes with the person's seed, as
/// the job carries it: for a host that got the request from elsewhere
/// (a browser from its platform).
pub fn issue_from(opener_seed: &[u8; 32], request: &JobTokenRequest) -> Result<JobToken, String> {
    let name = ai_member_name::read(&request.ai_member).to_string();
    let sealed = joy_token::job::issue(opener_seed, request)
        .map_err(|e| format!("{e}: joy auth delegation rotate {name}"))?;
    Ok(carried(request, sealed))
}

/// Issue the token of `job` in the name of the person `opener_key` and
/// seal it for the platform `domain`, for a host that holds their
/// identity seed: the delegation key derives from it and it seals the
/// token.
pub fn issue(
    project: &Project,
    job: &Item,
    opener_key: &str,
    opener_seed: &[u8; 32],
    domain: &str,
) -> Result<JobToken, String> {
    issue_from(opener_seed, &request(project, job, opener_key, domain)?)
}

/// Whether `token` can stand at `job` as the token `opener_key` issued
/// now: for a platform member of this project, sealed by that person,
/// holding no longer than a job's token does. What is inside the seal
/// only its platform reads ([`session`]).
pub fn accept(
    project: &Project,
    token: &JobToken,
    opener_key: &str,
    now: DateTime<Utc>,
) -> Result<(), String> {
    platform::key_of(project, &token.platform)?;
    if token.opener.id() != opener_key {
        return Err("the job's token names someone else as its opener".into());
    }
    let opener_key_hex = project
        .member_by_key(opener_key)
        .and_then(|member| member.verify_key.clone())
        .ok_or_else(|| format!("{opener_key} has no key in this project"))?;
    let sealed_by = joy_crypt::sealed::sealed_by(&token.sealed).map_err(|e| e.to_string())?;
    if sealed_by.to_hex() != opener_key_hex {
        return Err("the job's token is not sealed by the person who opens the job".into());
    }
    if token.expires <= now || token.expires > now + Duration::days(DAYS) + Duration::minutes(5) {
        return Err(format!(
            "a job's token holds {DAYS} days from now, not until {}",
            token.expires
        ));
    }
    Ok(())
}

/// Put `token` at the job: it runs on the token's platform from here on.
pub fn put(job: &mut Item, token: JobToken) {
    if let Some(spec) = job.job.as_mut() {
        spec.runs_on = Some(token.platform.clone());
        spec.token = Some(token);
    }
}

/// Take the token off the job; where it would run stays as a preset.
pub fn clear(job: &mut Item) {
    if let Some(spec) = job.job.as_mut() {
        spec.token = None;
    }
}

/// Take the job back from its platform: token and platform removed.
pub fn revoke(job: &mut Item) {
    if let Some(spec) = job.job.as_mut() {
        spec.token = None;
        spec.runs_on = None;
    }
}

/// The platform a job runs on, if it names one.
pub fn runs_on(job: &Item) -> Option<&str> {
    job.job.as_ref().and_then(|spec| spec.runs_on.as_deref())
}

/// The token of `job` in clear, for the platform `domain` that holds
/// `platform_seed`: or in words why the job has none this platform can
/// use.
fn unseal(
    project: &Project,
    job: &Item,
    domain: &str,
    platform_seed: &[u8; 32],
    now: DateTime<Utc>,
) -> Result<String, String> {
    let spec = job.job.as_ref().ok_or("this is not a job")?;
    if spec.runs_on.as_deref() != Some(domain) {
        return Err(format!("job {} does not run on {domain}", job.id));
    }
    let carried = spec
        .token
        .as_ref()
        .filter(|carried| carried.platform == domain)
        .ok_or_else(|| format!("job {} carries no token for {domain}", job.id))?;
    if carried.expires <= now {
        return Err(format!(
            "the job's token expired on {}",
            carried.expires.format("%Y-%m-%d")
        ));
    }
    let opener = carried.opener.id();
    let opener_key: PublicKey = project
        .member_by_key(opener)
        .and_then(|member| member.verify_key.as_deref())
        .and_then(|hex| PublicKey::from_hex(hex).ok())
        .ok_or_else(|| "the person who opened the job is no longer a member".to_string())?;
    let (sealed_by, plain) = joy_crypt::sealed::open_from(
        &carried.sealed,
        platform_seed,
        &info(&session::project_id_of(project), &job.id),
    )
    .map_err(|_| "the job's token cannot be opened with this platform's key".to_string())?;
    if sealed_by.to_hex() != opener_key.to_hex() {
        return Err("the job's token is not sealed by the person it names".into());
    }
    String::from_utf8(plain).map_err(|_| "the job's token is not a token".to_string())
}

/// The token of `job` for one round on the platform `domain`, in clear
/// and checked, with what redeeming it gives: the seal opened with the
/// platform's key, the token redeemed against the project as it stands,
/// and held to what the job says.
///
/// The reason of a refusal is what the job is sent back to new with:
/// the token expired or is not the opener's, their delegation was
/// rotated or removed, their grant changed since, or it is a token for
/// another job or member.
pub fn open(
    project: &Project,
    job: &Item,
    domain: &str,
    platform_seed: &[u8; 32],
) -> Result<(String, RedeemedSession), String> {
    let token_str = unseal(project, job, domain, platform_seed, Utc::now())?;
    let claims = token::decode_token(&token_str)
        .map_err(|e| e.to_string())?
        .claims;
    if token::job_scope(&claims) != Some(job.id.as_str()) {
        return Err("the job's token is for another job".into());
    }
    let assignee = ai_assignee(project, job)
        .ok_or_else(|| format!("job {} is not assigned to an AI member", job.id))?;
    let redeemed = redeem_ai_session(project, &session::project_id_of(project), &token_str)
        .map_err(|e| e.to_string())?;
    if redeemed.member != assignee {
        return Err("the job's token is for another member than its assignee".into());
    }
    // The grant is held against the person's entry here already, in the
    // words every refusal for it has: a job whose opener changed what
    // the AI may do for them goes back to new before a round is spent.
    super::grants::effective(
        project,
        &assignee,
        redeemed
            .token
            .claims
            .delegated_by
            .as_deref()
            .unwrap_or_default(),
        redeemed.token.claims.grant.as_deref(),
    )?;
    Ok((token_str, redeemed))
}

/// [`open`], for a caller that wants the session alone.
pub fn session(
    project: &Project,
    job: &Item,
    domain: &str,
    platform_seed: &[u8; 32],
) -> Result<RedeemedSession, String> {
    open(project, job, domain, platform_seed).map(|(_, redeemed)| redeemed)
}

/// The identity a session of a job's token acts with.
pub fn identity_of(redeemed: &RedeemedSession) -> Identity {
    let claims = &redeemed.token.claims;
    Identity {
        member: claims.member.clone().into(),
        delegated_by: claims.delegated_by.clone().map(Into::into),
        authenticated: true,
        grant: claims.grant.clone(),
        job: claims.job.clone(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::auth::vouch::{self, Occasion};
    use crate::auth::{delegation, generate_salt, IdentityKeypair};
    use crate::guard::{Action, Guard, Verdict};
    use crate::model::item::{Assignee, Capability, ItemType, JobSpec, Priority};
    use crate::model::project::{
        AiDelegationEntry, Member, MemberCapabilities, MemberFileShape, MemberLayout,
    };
    use joy_model::InteractionLevel;

    pub(crate) const FOUNDER: &str = "founder@example.com";
    pub(crate) const PLATFORM: &str = "app.example.com";
    pub(crate) const FOUNDER_SEED: [u8; 32] = [1u8; 32];
    pub(crate) const PLATFORM_SEED: [u8; 32] = [9u8; 32];
    const JOB: &str = "SH-JOB-0001-AA";

    fn caps(list: &[Capability]) -> MemberCapabilities {
        MemberCapabilities::Specific(list.iter().map(|c| (*c, Default::default())).collect())
    }

    /// A project with member files that say their type: the founder
    /// (manage, a delegation to claude), claude (implement, create, jobs
    /// at autonomous, signed) and the platform, signed for by the
    /// founder.
    pub(crate) fn project() -> Project {
        let mut project = Project::new("Shop".into(), Some("SH".into()));
        project.set_member_layout(MemberLayout::Files);
        project.set_member_file_shape(MemberFileShape::Identity);
        let founder_kp = IdentityKeypair::from_seed(&FOUNDER_SEED);
        let project_id = session::project_id_of(&project);

        let mut founder = Member::person(MemberCapabilities::All);
        founder.verify_key = Some(founder_kp.public_key().to_hex());
        let salt = generate_salt();
        let seed = delegation::derive_delegation_seed(&FOUNDER_SEED, &salt, &project_id, "claude");
        founder.put_delegation(
            "claude",
            AiDelegationEntry {
                delegation_verifier: IdentityKeypair::from_seed(&seed).public_key().to_hex(),
                delegation_salt: Some(salt.to_hex()),
                created: Utc::now(),
                rotated: None,
                grant: None,
                model: None,
            },
        );
        project.register_member(FOUNDER, founder).unwrap();

        let mut claude = Member::ai(caps(&[
            Capability::Implement,
            Capability::Create,
            Capability::Jobs,
        ]));
        claude.interaction_level = Some(InteractionLevel::Autonomous);
        vouch::sign(
            &project,
            FOUNDER,
            &founder_kp,
            "claude",
            &mut claude,
            Occasion::New,
        );
        project.register_member("claude", claude).unwrap();

        let key = IdentityKeypair::from_seed(&PLATFORM_SEED)
            .public_key()
            .to_hex();
        let mut member = platform::new_member(&key);
        vouch::sign(
            &project,
            FOUNDER,
            &founder_kp,
            PLATFORM,
            &mut member,
            Occasion::New,
        );
        project.register_member(PLATFORM, member).unwrap();
        project
    }

    pub(crate) fn job(status: Status) -> Item {
        let mut item = Item::new(
            JOB.into(),
            "a job".into(),
            ItemType::Job,
            Priority::Medium,
            Vec::new(),
        );
        item.status = status;
        item.assignees = vec![Assignee {
            member: "claude".into(),
            capabilities: Vec::new(),
        }];
        item.job = Some(JobSpec {
            scope: vec!["SH-0001".into()],
            budget: None,
            window: None,
            feedback: None,
            activity: Vec::new(),
            base_branch: None,
            result_branch: None,
            runs_on: None,
            token: None,
        });
        item
    }

    fn person() -> Identity {
        Identity::new(FOUNDER, None, true)
    }

    #[test]
    fn a_platform_member_is_what_a_person_signed_and_nothing_else() {
        let project = project();
        assert_eq!(platform::platforms(&project), vec![PLATFORM.to_string()]);
        let key = platform::key_of(&project, PLATFORM).unwrap();
        assert_eq!(
            key.to_hex(),
            IdentityKeypair::from_seed(&PLATFORM_SEED)
                .public_key()
                .to_hex()
        );

        // its key swapped after the signature: no platform at all
        let mut swapped = project.clone();
        swapped.member_by_key_mut(PLATFORM).unwrap().verify_key =
            Some(IdentityKeypair::from_seed(&[5u8; 32]).public_key().to_hex());
        assert_eq!(
            platform::key_of(&swapped, PLATFORM).unwrap_err(),
            "the platform app.example.com was changed without a signature for it"
        );
        // more capabilities than were signed: the same
        let mut widened = project.clone();
        widened
            .member_by_key_mut(PLATFORM)
            .unwrap()
            .set_capabilities(MemberCapabilities::All);
        assert!(platform::key_of(&widened, PLATFORM).is_err());
        // nobody signed
        let mut unsigned = project.clone();
        unsigned.member_by_key_mut(PLATFORM).unwrap().granted = None;
        assert_eq!(
            platform::key_of(&unsigned, PLATFORM).unwrap_err(),
            "nobody has signed for the platform app.example.com"
        );
        // and a person is not a platform
        assert!(platform::key_of(&project, FOUNDER).is_err());

        // the platform proves itself with the key that was signed for
        let own = IdentityKeypair::from_seed(&PLATFORM_SEED);
        assert_eq!(
            platform::identity(&project, PLATFORM, &own)
                .unwrap()
                .member
                .id(),
            PLATFORM
        );
        let other = IdentityKeypair::from_seed(&[5u8; 32]);
        assert!(platform::identity(&project, PLATFORM, &other).is_err());
    }

    #[test]
    fn the_platform_of_a_job_is_named_preset_or_the_only_one() {
        let mut project = project();
        assert_eq!(platform::for_job(&project, None, None).unwrap(), PLATFORM);
        assert_eq!(
            platform::for_job(&project, None, Some("other.example.com")).unwrap_err(),
            "other.example.com is not a platform member of this project"
        );
        let key = IdentityKeypair::from_seed(&[5u8; 32]).public_key().to_hex();
        project
            .register_member("two.example.com", platform::new_member(&key))
            .unwrap();
        assert_eq!(
            platform::for_job(&project, None, None).unwrap_err(),
            "this project has more than one platform, name the one that runs the job: \
             app.example.com, two.example.com"
        );
        assert_eq!(
            platform::for_job(&project, Some(PLATFORM), None).unwrap(),
            PLATFORM
        );
        assert_eq!(
            platform::for_job(&project, Some(PLATFORM), Some("two.example.com")).unwrap(),
            "two.example.com"
        );
        let mut none = project.clone();
        none.remove_member(PLATFORM);
        none.remove_member("two.example.com");
        assert_eq!(
            platform::for_job(&none, None, None).unwrap_err(),
            "no platform runs jobs for this project: sign in on a platform first"
        );
    }

    #[test]
    fn a_person_hands_a_job_over_and_nobody_else_does() {
        let project = project();
        let ai = Identity {
            member: "claude".into(),
            delegated_by: Some(FOUNDER.into()),
            authenticated: true,
            grant: None,
            job: None,
        };
        // approving it, starting it oneself, asking for changes
        assert!(handed_over(
            &project,
            &job(Status::Open),
            &Status::New,
            &person()
        ));
        assert!(handed_over(
            &project,
            &job(Status::InProgress),
            &Status::Open,
            &person()
        ));
        assert!(handed_over(
            &project,
            &job(Status::InProgress),
            &Status::Review,
            &person()
        ));
        // the AI taking its own job, or handing it to review, does not
        assert!(!handed_over(
            &project,
            &job(Status::InProgress),
            &Status::Open,
            &ai
        ));
        assert!(!handed_over(
            &project,
            &job(Status::Review),
            &Status::InProgress,
            &ai
        ));
        // nor a step that puts nobody to work
        assert!(!handed_over(
            &project,
            &job(Status::New),
            &Status::Open,
            &person()
        ));
        assert!(!handed_over(
            &project,
            &job(Status::Closed),
            &Status::Review,
            &person()
        ));
        assert!(!handed_over(
            &project,
            &job(Status::Open),
            &Status::InProgress,
            &person()
        ));
        // the answer to the AI's question does
        let mut answered = job(Status::Review);
        answered.job.as_mut().unwrap().feedback = Some(JobFeedback::Received);
        assert!(handed_over(&project, &answered, &Status::Review, &person()));
        // a job a person works needs no token
        let mut theirs = job(Status::Open);
        theirs.assignees[0].member = FOUNDER.into();
        assert!(!handed_over(&project, &theirs, &Status::New, &person()));
    }

    #[test]
    fn the_token_of_a_job_is_for_that_job_that_member_and_that_platform() {
        let project = project();
        let mut job = job(Status::Open);
        let token = issue(&project, &job, FOUNDER, &FOUNDER_SEED, PLATFORM).unwrap();
        assert_eq!(token.platform, PLATFORM);
        assert_eq!(token.opener.id(), FOUNDER);
        let days = (token.expires - Utc::now()).num_hours();
        assert!((DAYS * 24 - 1..=DAYS * 24).contains(&days), "{days}");
        accept(&project, &token, FOUNDER, Utc::now()).unwrap();
        put(&mut job, token);
        assert_eq!(runs_on(&job), Some(PLATFORM));

        // the platform opens it and gets a session of claude for the
        // founder, held to this job
        let redeemed = session(&project, &job, PLATFORM, &PLATFORM_SEED).unwrap();
        let identity = identity_of(&redeemed);
        assert_eq!(identity.member.id(), "claude");
        assert_eq!(
            identity.delegated_by.as_ref().map(|d| d.id()),
            Some(FOUNDER)
        );
        assert_eq!(identity.job.as_deref(), Some(JOB));
        assert!(identity.authenticated);

        // nobody else opens it
        assert_eq!(
            session(&project, &job, PLATFORM, &[5u8; 32]).err().unwrap(),
            "the job's token cannot be opened with this platform's key"
        );
        // and it is no token for another job
        let mut other = job.clone();
        other.id = "SH-JOB-0002-BB".into();
        assert!(session(&project, &other, PLATFORM, &PLATFORM_SEED).is_err());
        // nor for a job that runs elsewhere, or was taken back
        assert_eq!(
            session(&project, &job, "two.example.com", &PLATFORM_SEED)
                .err()
                .unwrap(),
            "job SH-JOB-0001-AA does not run on two.example.com"
        );
        let mut taken_back = job.clone();
        revoke(&mut taken_back);
        assert!(runs_on(&taken_back).is_none());
        assert!(session(&project, &taken_back, PLATFORM, &PLATFORM_SEED).is_err());
        // sent back to new, it keeps where it would run and loses the token
        let mut bounced = job.clone();
        clear(&mut bounced);
        assert_eq!(runs_on(&bounced), Some(PLATFORM));
        assert_eq!(
            session(&project, &bounced, PLATFORM, &PLATFORM_SEED)
                .err()
                .unwrap(),
            "job SH-JOB-0001-AA carries no token for app.example.com"
        );
    }

    #[test]
    fn a_token_that_no_longer_holds_is_refused_in_words() {
        let project = project();
        let mut job = job(Status::Open);
        let token = issue(&project, &job, FOUNDER, &FOUNDER_SEED, PLATFORM).unwrap();
        put(&mut job, token);

        // expired
        let mut expired = job.clone();
        expired
            .job
            .as_mut()
            .unwrap()
            .token
            .as_mut()
            .unwrap()
            .expires = Utc::now() - Duration::hours(1);
        assert!(session(&project, &expired, PLATFORM, &PLATFORM_SEED)
            .err()
            .unwrap()
            .starts_with("the job's token expired on "));

        // the opener set a grant of their own since: the token names the
        // grant it was issued with
        let mut granted = project.clone();
        crate::auth::grants::set_personal(
            &mut granted,
            "claude",
            FOUNDER,
            &IdentityKeypair::from_seed(&FOUNDER_SEED),
            None,
            Some(InteractionLevel::Proposing),
        )
        .unwrap();
        let why = session(&granted, &job, PLATFORM, &PLATFORM_SEED)
            .err()
            .unwrap();
        assert!(
            why.contains("has changed since its token was issued"),
            "{why}"
        );
        // a token issued after it holds
        let mut fresh = job.clone();
        let token = issue(&granted, &fresh, FOUNDER, &FOUNDER_SEED, PLATFORM).unwrap();
        put(&mut fresh, token);
        session(&granted, &fresh, PLATFORM, &PLATFORM_SEED).unwrap();

        // the delegation is gone
        let mut undelegated = project.clone();
        undelegated
            .member_by_key_mut(FOUNDER)
            .unwrap()
            .ai_delegations
            .clear();
        assert!(session(&undelegated, &job, PLATFORM, &PLATFORM_SEED).is_err());

        // somebody else put a token there under the opener's name
        let mut planted = job.clone();
        let theirs = joy_crypt::sealed::seal_for(
            &[5u8; 32],
            &platform::key_of(&project, PLATFORM).unwrap(),
            &info(&session::project_id_of(&project), JOB),
            b"joy_t_anything",
        );
        planted.job.as_mut().unwrap().token.as_mut().unwrap().sealed = theirs;
        assert_eq!(
            session(&project, &planted, PLATFORM, &PLATFORM_SEED)
                .err()
                .unwrap(),
            "the job's token is not sealed by the person it names"
        );
        let token = planted.job.as_ref().unwrap().token.as_ref().unwrap();
        assert_eq!(
            accept(&project, token, FOUNDER, Utc::now()).unwrap_err(),
            "the job's token is not sealed by the person who opens the job"
        );
    }

    #[test]
    fn without_a_delegation_or_a_platform_no_token_is_issued() {
        let project = project();
        let job = job(Status::New);
        let mut undelegated = project.clone();
        undelegated
            .member_by_key_mut(FOUNDER)
            .unwrap()
            .ai_delegations
            .clear();
        assert_eq!(
            issue(&undelegated, &job, FOUNDER, &FOUNDER_SEED, PLATFORM).unwrap_err(),
            "you have not delegated to claude yet: joy auth token add claude"
        );
        assert!(issue(
            &project,
            &job,
            FOUNDER,
            &FOUNDER_SEED,
            "nowhere.example.com"
        )
        .is_err());
        // a project whose member files do not say their type yet
        let mut older = project.clone();
        older.set_member_file_shape(MemberFileShape::Flat);
        assert_eq!(
            issue(&older, &job, FOUNDER, &FOUNDER_SEED, PLATFORM).unwrap_err(),
            "A platform that runs jobs needs an update of this project."
        );
    }

    /// The guard on disk: a session of a job's token writes the job and
    /// what it names, the platform writes its two things.
    #[test]
    fn the_guard_holds_a_job_session_and_a_platform_to_what_they_write() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(crate::store::joy_dir(root).join("logs")).unwrap();
        let project = project();
        crate::store::save_project(root, &project).unwrap();
        let item = |id: &str, parent: Option<&str>| {
            let mut item = Item::new(
                id.into(),
                format!("item {id}"),
                ItemType::Task,
                Priority::Medium,
                Vec::new(),
            );
            item.parent = parent.map(str::to_string);
            crate::items::save_item(root, &item).unwrap();
        };
        item("SH-0001", None);
        item("SH-0002", Some("SH-0001"));
        item("SH-0003", None);
        let mut job = job(Status::Open);
        let token = issue(&project, &job, FOUNDER, &FOUNDER_SEED, PLATFORM).unwrap();
        put(&mut job, token);
        crate::items::save_item(root, &job).unwrap();
        // the first job on a platform lifts the format
        let said =
            std::fs::read_to_string(crate::store::joy_dir(root).join("project.yaml")).unwrap();
        assert!(said.starts_with("format: 5\n"), "{said}");

        let project = crate::store::load_project(root).unwrap();
        let job = crate::items::load_jobs(root).unwrap().remove(0);
        let guard = Guard::load(root).unwrap();
        let claude = identity_of(&session(&project, &job, PLATFORM, &PLATFORM_SEED).unwrap());
        let take = Action::ChangeJobStatus {
            from: Status::Open,
            to: Status::InProgress,
            assignee: Some("claude".into()),
        };
        // its job, a scope item and an item under one
        assert_eq!(guard.check_at(&take, JOB, &claude), Verdict::Allow);
        assert_eq!(
            guard.check_at(&Action::AddComment, JOB, &claude),
            Verdict::Allow
        );
        assert_eq!(
            guard.check_at(&Action::AddComment, "SH-0001", &claude),
            Verdict::Allow
        );
        assert_eq!(
            guard.check_at(&Action::UpdateItem, "SH-0002", &claude),
            Verdict::Allow
        );
        // nothing else, and nothing new
        assert_eq!(
            guard.check_at(&Action::AddComment, "SH-0003", &claude),
            Verdict::Deny(
                "this session is for job SH-JOB-0001-AA, and SH-0003 is not part of it".into()
            )
        );
        assert!(guard
            .check_at(&Action::CreateItem, "SH-0004", &claude)
            .is_denied());
        assert!(guard.check(&Action::AddComment, &claude).is_denied());
        // the same member with a token for everything is not held to a job
        let free = Identity {
            job: None,
            ..claude.clone()
        };
        assert_eq!(
            guard.check_at(&Action::AddComment, "SH-0003", &free),
            Verdict::Allow
        );

        // the platform: its own job back to new, a comment on a job
        let own = platform::identity(
            &project,
            PLATFORM,
            &IdentityKeypair::from_seed(&PLATFORM_SEED),
        )
        .unwrap();
        let back = Action::ChangeJobStatus {
            from: Status::Open,
            to: Status::New,
            assignee: Some("claude".into()),
        };
        assert_eq!(guard.check_at(&back, JOB, &own), Verdict::Allow);
        assert_eq!(
            guard.check_at(&Action::AddComment, JOB, &own),
            Verdict::Allow
        );
        let refusal = Verdict::Deny(
            "the platform app.example.com only sends its own jobs back to new and comments on a job"
                .into(),
        );
        assert_eq!(guard.check_at(&take, JOB, &own), refusal);
        assert_eq!(
            guard.check_at(&Action::AddComment, "SH-0001", &own),
            refusal
        );
        assert_eq!(
            guard.check_at(&Action::ManageProject, "project", &own),
            refusal
        );
        assert_eq!(guard.check(&back, &own), refusal);
        // a job taken back is no longer its own, but it may still say
        // what it left there
        let mut taken_back = job.clone();
        revoke(&mut taken_back);
        crate::items::save_item(root, &taken_back).unwrap();
        assert_eq!(guard.check_at(&back, JOB, &own), refusal);
        assert_eq!(
            guard.check_at(&Action::AddComment, JOB, &own),
            Verdict::Allow
        );
        // without proof of its key it is nobody
        let unproven = Identity {
            authenticated: false,
            ..own
        };
        assert!(guard
            .check_at(&Action::AddComment, JOB, &unproven)
            .is_denied());
    }
}
