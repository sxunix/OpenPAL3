mod _let;
mod appraise;
pub(crate) use appraise::play_sfx;
mod between;
mod call;
mod camera_anim;
mod camera_default;
mod camera_focus;
mod camera_move;
mod camera_set;
mod cave_wolf;
mod climb;
mod cmp;
mod combat;
mod combat_effect;
mod dealer;
mod dealer_layout;
mod dlg;
mod dlg_face;
pub(crate) mod dlg_sel;
mod dlg_time;
mod encamp;
mod entry_row;
mod fade_in;
mod fade_in_white;
mod fade_out;
mod fade_out_white;
mod fop;
mod get_appr;
mod get_combat;
mod get_dlg_sel;
mod get_money;
mod get_time_sel;
mod goto;
mod have_item;
mod hockshop;
mod hy_fly;
mod idle;
mod if_in_team;
mod interact;
mod inventory;
mod item_preview;
mod killflyer;
mod load_scene;
mod main_menu;
mod minigames;
mod movie;
mod music;
mod nop;
mod object_active;
mod open_door;
mod party;
mod play_sound;
mod quake;
mod rnd;
mod role_act_auto_stand;
mod role_active;
mod role_ctrl;
mod role_emote;
mod role_end_action;
mod role_face_role;
pub mod role_fade;
mod role_input;
mod role_move_back;
mod role_move_to;
mod role_path_out;
mod role_path_to;
mod role_script;
mod role_set_face;
mod role_set_layer;
mod role_set_pos;
mod role_show_action;
mod role_stop;
mod role_turn_face;
mod scene_state;
mod screen_effect;
mod script_run_mode;
mod set_bigmap_element;
mod show_chat_rest;
mod ski;
mod smelting;
mod snow;
mod start_hidefight;
mod stop_music;
mod story_flags;
mod switch_mode;
mod team;
mod testgoto;
mod text_ui;
mod ui_sound;
pub mod vars;
mod water_magic;
mod world_rotate;
mod xj_pic;

pub use _let::{SceCommandLet, SceCommandLetAdd};
pub use appraise::SceCommandAppraise;
pub use between::SceCommandBetween;
pub use call::SceCommandCall;
pub use camera_anim::{
    SceCommandCameraOrbit, SceCommandCameraPush, SceCommandCameraSwing, SceCommandCameraYaw,
};
pub use camera_default::SceCommandCameraDefault;
pub use camera_focus::{
    SceCommandCameraFocusObject, SceCommandCameraFocusPoint, SceCommandCameraFocusRole,
    SceCommandCameraFollowPlayer, SceCommandCameraPopState, SceCommandCameraPushState,
};
pub use camera_move::SceCommandCameraMove;
pub use camera_set::SceCommandCameraSet;
pub use cave_wolf::{SceCommandMiniGameCaveExperience, take_start_request as take_cave_wolf_start};
pub use climb::SceCommandClimb;
pub use cmp::{
    SceCommandEq, SceCommandGeq, SceCommandGeq2, SceCommandGt, SceCommandLeq, SceCommandLs,
    SceCommandNeq,
};
pub use combat::{
    SceCommandCombat, SceCommandCombatMaxRound, SceCommandCombatMustFail,
    SceCommandCombatNotGameOver, SceCommandGameOverScreen,
};
pub use combat_effect::{SceCommandPlayCombatEffect, SceCommandPreloadCombatEffect};
pub use dealer::SceCommandDealerMenu;
pub use dlg::SceCommandDlg;
pub use dlg_face::SceCommandDlgFace;
pub use dlg_sel::SceCommandDlgSel;
pub use dlg_time::SceCommandDlgTime;
pub use encamp::SceCommandEncamp;
pub use entry_row::SceCommandEntryRow;
pub use fade_in::SceCommandFadeIn;
pub use fade_in_white::SceCommandFadeInWhite;
pub use fade_out::SceCommandFadeOut;
pub use fade_out_white::SceCommandFadeOutWhite;
pub use fop::SceCommandFop;
pub use get_appr::SceCommandGetAppr;
pub use get_combat::SceCommandGetCombat;
pub use get_dlg_sel::SceCommandGetDlgSel;
pub use get_money::SceCommandGetMoney;
pub use get_time_sel::SceCommandGetTimeSel;
pub use goto::SceCommandGoto;
pub use have_item::SceCommandHaveItem;
pub use hockshop::SceCommandMiniGamePawnshop;
pub use hy_fly::SceCommandHyFly;
pub use idle::SceCommandIdle;
pub use if_in_team::SceCommandIfInTeam;
pub use interact::SceCommandInteractWithObject;
pub use inventory::{SceCommandAddItem, SceCommandAddMoney, SceCommandRemoveItem};
pub use killflyer::SceCommandMiniGameSwatAFly;
pub use load_scene::SceCommandLoadScene;
pub use main_menu::SceCommandSwitchToMainMenu;
pub use minigames::{
    SceCommandEffectAtRole, SceCommandEffectPosition, SceCommandGetPawnshopAccrual,
    SceCommandGetPawnshopLevel, SceCommandGetPawnshopManage, SceCommandGetPawnshopPeriod,
    SceCommandGetPawnshopUnHock, SceCommandMiniGamePawnshopNote, SceCommandNavigationBeeFlyTo,
};
pub use movie::SceCommandMovie;
pub use music::SceCommandMusic;
pub use nop::SceCommandNop;
pub use object_active::SceCommandObjectActive;
pub use open_door::SceCommandOpenDoor;
pub use party::{
    SceCommandAddSkill, SceCommandFavorAdd, SceCommandFullRoleAtt, SceCommandFullTeamAtt,
    SceCommandGetFavor, SceCommandGetFavorite, SceCommandInTeam, SceCommandSetRoleAtt,
};
pub use play_sound::SceCommandPlaySound;
pub use quake::SceCommandQuake;
pub use rnd::SceCommandRnd;
pub use role_act_auto_stand::SceCommandRoleActAutoStand;
pub use role_active::SceCommandRoleActive;
pub use role_ctrl::SceCommandRoleCtrl;
pub use role_emote::SceCommandRoleEmote;
pub use role_end_action::SceCommandRoleEndAction;
pub use role_face_role::SceCommandRoleFaceRole;
pub use role_fade::{SceCommandRoleFadeIn, SceCommandRoleFadeOut};
pub use role_input::SceCommandRoleInput;
pub use role_move_back::SceCommandRoleMoveBack;
pub use role_move_to::SceCommandRoleMoveTo;
pub use role_path_out::SceCommandRolePathOut;
pub use role_path_to::SceCommandRolePathTo;
pub use role_script::SceCommandRoleScript;
pub use role_set_face::SceCommandRoleSetFace;
pub use role_set_layer::SceCommandRoleSetLayer;
pub use role_set_pos::SceCommandRoleSetPos;
pub use role_show_action::SceCommandRoleShowAction;
pub use role_stop::SceCommandRoleStop;
pub use role_turn_face::SceCommandRoleTurnFace;
pub use scene_state::{
    SceCommandGetSwitch, SceCommandInitFlower, SceCommandObjectMove, SceCommandObjectNotFromSave,
};
pub use screen_effect::SceCommandScreenEffect;
pub use script_run_mode::SceCommandScriptRunMode;
pub use set_bigmap_element::SceCommandSetBigMapElement;
pub use show_chat_rest::SceCommandShowChatRest;
pub use ski::SceCommandMiniGameSki;
pub use smelting::SceCommandSmeltingMenu;
pub use snow::SceCommandSnow;
pub use start_hidefight::SceCommandStartHideFight;
pub use stop_music::SceCommandStopMusic;
pub use story_flags::longkui_model;
pub use story_flags::{
    SceCommandAverageLevel, SceCommandEnableFeature, SceCommandGiveCloth, SceCommandHuaYingMode,
    SceCommandLearnCompose, SceCommandLoadAction, SceCommandLongKuiMode, SceCommandRoleOverlap,
    SceCommandRoleScale, SceCommandSetRoleTexture, SceCommandVarDistract, apply_longkui_model,
};
pub use switch_mode::SceCommandSwitchMode;
pub use team::{
    SceCommandTeamClose, SceCommandTeamCloseOnSceneChange, SceCommandTeamOpen,
    SceCommandTeamOpenOnSceneChange,
};
pub use testgoto::SceCommandTestGoto;
pub use text_ui::{SceCommandCaption, SceCommandNote};
pub use water_magic::{SceCommandWaterMagic, WATER_MAGIC_PAR};
pub use world_rotate::{SceCommandRotateWorld, SceCommandRotateWorldBack};
pub use xj_pic::SceCommandShowXueJianPicture;

use radiance::math::Vec3;

struct Direction;
impl Direction {
    const NORTH: Vec3 = Vec3 {
        x: 0.,
        y: 0.,
        z: -1.,
    };
    const NORTHEAST: Vec3 = Vec3 {
        x: 1.,
        y: 0.,
        z: -1.,
    };
    const EAST: Vec3 = Vec3 {
        x: 1.,
        y: 0.,
        z: 0.,
    };
    const SOUTHEAST: Vec3 = Vec3 {
        x: 1.,
        y: 0.,
        z: 1.,
    };
    const SOUTH: Vec3 = Vec3 {
        x: 0.,
        y: 0.,
        z: 1.,
    };
    const SOUTHWEST: Vec3 = Vec3 {
        x: -1.,
        y: 0.,
        z: 1.,
    };
    const WEST: Vec3 = Vec3 {
        x: -1.,
        y: 0.,
        z: 0.,
    };
    const NORTHWEST: Vec3 = Vec3 {
        x: -1.,
        y: 0.,
        z: -1.,
    };
}

/// 给 `SceState` 的浮层用（提示框）。
pub(crate) use appraise::overlay;
