// © 2019, ETH Zurich
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use syntax::ast::NodeId;
use rustc::hir;
use rustc::mir;
use rustc::hir::intravisit::*;
use syntax::codemap::Span;
use rustc::ty;
use rustc::ty::subst::Substs;
use validators::SupportStatus;
use validators::Reason;
use rustc::hir::def_id::DefId;
use std::collections::HashSet;
use prusti_interface::environment::{ProcedureLoops, Procedure};
use rustc::mir::interpret::GlobalId;
use rustc::middle::const_val::ConstVal;
use validators::unsafety_validator::contains_unsafe;
use validators::common_validator::CommonValidator;

pub struct PureFunctionValidator<'a, 'tcx: 'a> {
    tcx: ty::TyCtxt<'a, 'tcx, 'tcx>,
    support: SupportStatus,
    visited_inner_type_variants: HashSet<&'tcx ty::TypeVariants<'tcx>>
}

macro_rules! skip_visited_inner_type_variant {
    ($self:expr, $tv:expr) => {
        if $self.visited_inner_type_variants.contains(&$tv) {
            return;
        } else {
            $self.visited_inner_type_variants.insert($tv);
        }
    };
}

impl<'a, 'tcx: 'a> CommonValidator<'a, 'tcx> for PureFunctionValidator<'a, 'tcx> {
    fn support(&mut self) -> &mut SupportStatus {
        &mut self.support
    }

    fn get_support_status(self) -> SupportStatus {
        self.support
    }

    fn tcx(&self) -> ty::TyCtxt<'a, 'tcx, 'tcx> {
        self.tcx
    }

    fn check_inner_ty(&mut self, ty: ty::Ty<'tcx>, span: Span) {
        skip_visited_inner_type_variant!(self, &ty.sty);

        self.check_ty(ty, span);

        match ty.sty {
            ty::TypeVariants::TyRef(..) => unsupported!(self, span, "uses reference-typed fields"),

            _ => {} // OK
        }
    }
}

impl<'a, 'tcx: 'a> PureFunctionValidator<'a, 'tcx> {
    pub fn new(tcx: ty::TyCtxt<'a, 'tcx, 'tcx>) -> Self {
        PureFunctionValidator {
            tcx,
            support: SupportStatus::new(),
            visited_inner_type_variants: HashSet::new()
        }
    }

    pub fn check(&mut self, def_id: DefId) {
        let node_id = self.tcx.hir.as_local_node_id(def_id).unwrap();
        let span = self.tcx.def_span(def_id);

        let sig = self.tcx.fn_sig(def_id);
        self.check_fn_sig(sig.skip_binder(), def_id);

        let fn_node = self.tcx.hir.get(node_id);
        self.check_hir(fn_node);

        if contains_unsafe(self.tcx, node_id) {
            unsupported!(self, span, "contains unsafe code");
        }

        let procedure = Procedure::new(self.tcx, def_id);
        self.check_mir(&procedure);
    }

    fn check_fn_sig(&mut self, sig: &ty::FnSig<'tcx>, def_id: DefId) {
        let span = self.tcx.def_span(def_id);

        if sig.variadic {
            unsupported!(self, span, "is a C-variadic function");
        }

        match sig.unsafety {
            hir::Unsafety::Unsafe => {
                unsupported!(self, span, "is an unsafe function");
            }

            hir::Unsafety::Normal => {} // OK
        }

        // Note: the types are already checked from MIR
    }

    fn check_return_ty(&mut self, ty: ty::Ty<'tcx>, span: Span) {
        match ty.sty {
            ty::TypeVariants::TyBool => {} // OK

            ty::TypeVariants::TyChar => {} // OK

            ty::TypeVariants::TyInt(_) => {} // OK

            ty::TypeVariants::TyUint(_) => {} // OK

            _ => unsupported!(self, span, "has return value of type non-integer, non-boolean or non-char"),
        }
    }

    fn check_mir(&mut self, procedure: &Procedure<'a, 'tcx>) {
        self.check_mir_signature(procedure);

        let mir = procedure.get_mir();

        //for local_decl in &mir.local_decls {
        //    self.check_ty(local_decl.ty);
        //}

        if ProcedureLoops::new(mir).count_loop_heads() > 0 {
            unsupported!(self, procedure.get_span(), "uses loops");
        }

        // TODO: check only blocks that may lead to a `Return` terminator
        for (index, basic_block_data) in mir.basic_blocks().iter_enumerated() {
            if !procedure.is_reachable_block(index) || procedure.is_spec_block(index) {
                continue;
            }
            for stmt in &basic_block_data.statements {
                self.check_mir_stmt(mir, stmt);
            }
            self.check_mir_terminator(mir, basic_block_data.terminator.as_ref().unwrap());
        }
    }

    fn check_mir_signature(&mut self, procedure: &Procedure<'a, 'tcx>) {
        let mir = procedure.get_mir();
        let span = procedure.get_span();

        self.check_return_ty(mir.return_ty(), span);

        if !mir.yield_ty.is_none() {
            unsupported!(self, span, "uses `yield`");
        }
        if !mir.upvar_decls.is_empty() {
            unsupported!(self, span, "uses variables captured in closures");
        }

        for arg_index in mir.args_iter() {
            let arg = &mir.local_decls[arg_index];
            self.check_ty(arg.ty, arg.source_info.span);
        }
    }
}
