// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

#[cfg(test)]
mod instruction_tests;

#[cfg(test)]
mod tests;

use qsc_data_structures::target::TargetCapabilityFlags;
use qsc_lowerer::map_hir_package_to_fir;
use qsc_partial_eval::{partially_evaluate, ProgramEntry};
use qsc_rca::PackageStoreComputeProperties;
use qsc_rir::{
    passes::check_and_transform,
    rir::{self, ConditionCode},
    utils::get_all_block_successors,
};

fn lower_store(package_store: &qsc_frontend::compile::PackageStore) -> qsc_fir::fir::PackageStore {
    let mut fir_store = qsc_fir::fir::PackageStore::new();
    for (id, unit) in package_store {
        let package = qsc_lowerer::Lowerer::new().lower_package(&unit.package);
        fir_store.insert(map_hir_package_to_fir(id), package);
    }
    fir_store
}

/// converts the given sources to QIR using the given language features.
pub fn hir_to_qir(
    package_store: &qsc_frontend::compile::PackageStore,
    capabilities: TargetCapabilityFlags,
    compute_properties: Option<PackageStoreComputeProperties>,
    entry: &ProgramEntry,
) -> Result<String, qsc_partial_eval::Error> {
    let fir_store = lower_store(package_store);
    fir_to_qir(&fir_store, capabilities, compute_properties, entry)
}

pub fn fir_to_qir(
    fir_store: &qsc_fir::fir::PackageStore,
    capabilities: TargetCapabilityFlags,
    compute_properties: Option<PackageStoreComputeProperties>,
    entry: &ProgramEntry,
) -> Result<String, qsc_partial_eval::Error> {
    let mut program = get_rir_from_compilation(fir_store, compute_properties, entry, capabilities)?;
    check_and_transform(&mut program);
    Ok(ToQir::<String>::to_qir(&program, &program))
}

fn get_rir_from_compilation(
    fir_store: &qsc_fir::fir::PackageStore,
    compute_properties: Option<PackageStoreComputeProperties>,
    entry: &ProgramEntry,
    capabilities: TargetCapabilityFlags,
) -> Result<rir::Program, qsc_partial_eval::Error> {
    let compute_properties = compute_properties.unwrap_or_else(|| {
        let analyzer = qsc_rca::Analyzer::init(fir_store);
        analyzer.analyze_all()
    });

    partially_evaluate(fir_store, &compute_properties, entry, capabilities)
}

/// A trait for converting a type into QIR of type `T`.
/// This can be used to generate QIR strings or other representations.
pub trait ToQir<T> {
    fn to_qir(&self, program: &rir::Program) -> T;
}

impl ToQir<String> for rir::Literal {
    fn to_qir(&self, _program: &rir::Program) -> String {
        match self {
            rir::Literal::Bool(b) => format!("i1 {b}"),
            rir::Literal::Double(d) => {
                if (d.floor() - d.ceil()).abs() < f64::EPSILON {
                    // The value is a whole number, which requires at least one decimal point
                    // to differentiate it from an integer value.
                    format!("double {d:.1}")
                } else {
                    format!("double {d}")
                }
            }
            rir::Literal::Integer(i) => format!("i64 {i}"),
            rir::Literal::Pointer => "i8* null".to_string(),
            rir::Literal::Qubit(q) => format!("%Qubit* inttoptr (i64 {q} to %Qubit*)"),
            rir::Literal::Result(r) => format!("%Result* inttoptr (i64 {r} to %Result*)"),
        }
    }
}

impl ToQir<String> for rir::Ty {
    fn to_qir(&self, _program: &rir::Program) -> String {
        match self {
            rir::Ty::Boolean => "i1".to_string(),
            rir::Ty::Double => "double".to_string(),
            rir::Ty::Integer => "i64".to_string(),
            rir::Ty::Pointer => "i8*".to_string(),
            rir::Ty::Qubit => "%Qubit*".to_string(),
            rir::Ty::Result => "%Result*".to_string(),
        }
    }
}

impl ToQir<String> for Option<rir::Ty> {
    fn to_qir(&self, program: &rir::Program) -> String {
        match self {
            Some(ty) => ToQir::<String>::to_qir(ty, program),
            None => "void".to_string(),
        }
    }
}

impl ToQir<String> for rir::VariableId {
    fn to_qir(&self, _program: &rir::Program) -> String {
        format!("%var_{}", self.0)
    }
}

impl ToQir<String> for rir::Variable {
    fn to_qir(&self, program: &rir::Program) -> String {
        format!(
            "{} {}",
            ToQir::<String>::to_qir(&self.ty, program),
            ToQir::<String>::to_qir(&self.id, program)
        )
    }
}

impl ToQir<String> for rir::Operand {
    fn to_qir(&self, program: &rir::Program) -> String {
        match self {
            rir::Operand::Literal(lit) => ToQir::<String>::to_qir(lit, program),
            rir::Operand::Variable(var) => ToQir::<String>::to_qir(var, program),
        }
    }
}

impl ToQir<String> for rir::ConditionCode {
    fn to_qir(&self, _program: &rir::Program) -> String {
        match self {
            rir::ConditionCode::Eq => "eq".to_string(),
            rir::ConditionCode::Ne => "ne".to_string(),
            rir::ConditionCode::Sgt => "sgt".to_string(),
            rir::ConditionCode::Sge => "sge".to_string(),
            rir::ConditionCode::Slt => "slt".to_string(),
            rir::ConditionCode::Sle => "sle".to_string(),
        }
    }
}

impl ToQir<String> for rir::Instruction {
    fn to_qir(&self, program: &rir::Program) -> String {
        match self {
            rir::Instruction::Add(lhs, rhs, variable) => {
                binop_to_qir("add", lhs, rhs, *variable, program)
            }
            rir::Instruction::Ashr(lhs, rhs, variable) => {
                binop_to_qir("ashr", lhs, rhs, *variable, program)
            }
            rir::Instruction::BitwiseAnd(lhs, rhs, variable) => {
                simple_bitwise_to_qir("and", lhs, rhs, *variable, program)
            }
            rir::Instruction::BitwiseNot(value, variable) => {
                bitwise_not_to_qir(value, *variable, program)
            }
            rir::Instruction::BitwiseOr(lhs, rhs, variable) => {
                simple_bitwise_to_qir("or", lhs, rhs, *variable, program)
            }
            rir::Instruction::BitwiseXor(lhs, rhs, variable) => {
                simple_bitwise_to_qir("xor", lhs, rhs, *variable, program)
            }
            rir::Instruction::Branch(cond, true_id, false_id) => {
                format!(
                    "  br {}, label %{}, label %{}",
                    ToQir::<String>::to_qir(cond, program),
                    ToQir::<String>::to_qir(true_id, program),
                    ToQir::<String>::to_qir(false_id, program)
                )
            }
            rir::Instruction::Call(call_id, args, output) => {
                call_to_qir(args, *call_id, *output, program)
            }
            rir::Instruction::LogicalAnd(lhs, rhs, variable) => {
                logical_binop_to_qir("and", lhs, rhs, *variable, program)
            }
            rir::Instruction::LogicalNot(value, variable) => {
                logical_not_to_qir(value, *variable, program)
            }
            rir::Instruction::LogicalOr(lhs, rhs, variable) => {
                logical_binop_to_qir("or", lhs, rhs, *variable, program)
            }
            rir::Instruction::Mul(lhs, rhs, variable) => {
                binop_to_qir("mul", lhs, rhs, *variable, program)
            }
            rir::Instruction::Icmp(op, lhs, rhs, variable) => {
                icmp_to_qir(*op, lhs, rhs, *variable, program)
            }
            rir::Instruction::Jump(block_id) => {
                format!("  br label %{}", ToQir::<String>::to_qir(block_id, program))
            }
            rir::Instruction::Phi(args, variable) => phi_to_qir(args, *variable, program),
            rir::Instruction::Return => "  ret void".to_string(),
            rir::Instruction::Sdiv(lhs, rhs, variable) => {
                binop_to_qir("sdiv", lhs, rhs, *variable, program)
            }
            rir::Instruction::Shl(lhs, rhs, variable) => {
                binop_to_qir("shl", lhs, rhs, *variable, program)
            }
            rir::Instruction::Srem(lhs, rhs, variable) => {
                binop_to_qir("srem", lhs, rhs, *variable, program)
            }
            rir::Instruction::Store(_, _) => unimplemented!("store should be removed by pass"),
            rir::Instruction::Sub(lhs, rhs, variable) => {
                binop_to_qir("sub", lhs, rhs, *variable, program)
            }
        }
    }
}

fn logical_not_to_qir(
    value: &rir::Operand,
    variable: rir::Variable,
    program: &rir::Program,
) -> String {
    let value_ty = get_value_ty(value);
    let var_ty = get_variable_ty(variable);
    assert_eq!(
        value_ty, var_ty,
        "mismatched input/output types ({value_ty}, {var_ty}) for not"
    );
    assert_eq!(var_ty, "i1", "unsupported type {var_ty} for not");

    format!(
        "  {} = xor i1 {}, true",
        ToQir::<String>::to_qir(&variable.id, program),
        get_value_as_str(value, program)
    )
}

fn logical_binop_to_qir(
    op: &str,
    lhs: &rir::Operand,
    rhs: &rir::Operand,
    variable: rir::Variable,
    program: &rir::Program,
) -> String {
    let lhs_ty = get_value_ty(lhs);
    let rhs_ty = get_value_ty(rhs);
    let var_ty = get_variable_ty(variable);
    assert_eq!(
        lhs_ty, rhs_ty,
        "mismatched input types ({lhs_ty}, {rhs_ty}) for {op}"
    );
    assert_eq!(
        lhs_ty, var_ty,
        "mismatched input/output types ({lhs_ty}, {var_ty}) for {op}"
    );
    assert_eq!(var_ty, "i1", "unsupported type {var_ty} for {op}");

    format!(
        "  {} = {op} {var_ty} {}, {}",
        ToQir::<String>::to_qir(&variable.id, program),
        get_value_as_str(lhs, program),
        get_value_as_str(rhs, program)
    )
}

fn bitwise_not_to_qir(
    value: &rir::Operand,
    variable: rir::Variable,
    program: &rir::Program,
) -> String {
    let value_ty = get_value_ty(value);
    let var_ty = get_variable_ty(variable);
    assert_eq!(
        value_ty, var_ty,
        "mismatched input/output types ({value_ty}, {var_ty}) for not"
    );
    assert_eq!(var_ty, "i64", "unsupported type {var_ty} for not");

    format!(
        "  {} = xor {var_ty} {}, -1",
        ToQir::<String>::to_qir(&variable.id, program),
        get_value_as_str(value, program)
    )
}

fn call_to_qir(
    args: &[rir::Operand],
    call_id: rir::CallableId,
    output: Option<rir::Variable>,
    program: &rir::Program,
) -> String {
    let args = args
        .iter()
        .map(|arg| ToQir::<String>::to_qir(arg, program))
        .collect::<Vec<_>>()
        .join(", ");
    let callable = program.get_callable(call_id);
    if let Some(output) = output {
        format!(
            "  {} = call {} @{}({args})",
            ToQir::<String>::to_qir(&output.id, program),
            ToQir::<String>::to_qir(&callable.output_type, program),
            callable.name
        )
    } else {
        format!(
            "  call {} @{}({args})",
            ToQir::<String>::to_qir(&callable.output_type, program),
            callable.name
        )
    }
}

fn icmp_to_qir(
    op: ConditionCode,
    lhs: &rir::Operand,
    rhs: &rir::Operand,
    variable: rir::Variable,
    program: &rir::Program,
) -> String {
    let lhs_ty = get_value_ty(lhs);
    let rhs_ty = get_value_ty(rhs);
    let var_ty = get_variable_ty(variable);
    assert_eq!(
        lhs_ty, rhs_ty,
        "mismatched input types ({lhs_ty}, {rhs_ty}) for icmp {op}"
    );

    assert_eq!(var_ty, "i1", "unsupported output type {var_ty} for icmp");
    format!(
        "  {} = icmp {} {lhs_ty} {}, {}",
        ToQir::<String>::to_qir(&variable.id, program),
        ToQir::<String>::to_qir(&op, program),
        get_value_as_str(lhs, program),
        get_value_as_str(rhs, program)
    )
}

fn binop_to_qir(
    op: &str,
    lhs: &rir::Operand,
    rhs: &rir::Operand,
    variable: rir::Variable,
    program: &rir::Program,
) -> String {
    let lhs_ty = get_value_ty(lhs);
    let rhs_ty = get_value_ty(rhs);
    let var_ty = get_variable_ty(variable);
    assert_eq!(
        lhs_ty, rhs_ty,
        "mismatched input types ({lhs_ty}, {rhs_ty}) for {op}"
    );
    assert_eq!(
        lhs_ty, var_ty,
        "mismatched input/output types ({lhs_ty}, {var_ty}) for {op}"
    );
    assert_eq!(var_ty, "i64", "unsupported type {var_ty} for {op}");

    format!(
        "  {} = {op} {var_ty} {}, {}",
        ToQir::<String>::to_qir(&variable.id, program),
        get_value_as_str(lhs, program),
        get_value_as_str(rhs, program)
    )
}

fn simple_bitwise_to_qir(
    op: &str,
    lhs: &rir::Operand,
    rhs: &rir::Operand,
    variable: rir::Variable,
    program: &rir::Program,
) -> String {
    let lhs_ty = get_value_ty(lhs);
    let rhs_ty = get_value_ty(rhs);
    let var_ty = get_variable_ty(variable);
    assert_eq!(
        lhs_ty, rhs_ty,
        "mismatched input types ({lhs_ty}, {rhs_ty}) for {op}"
    );
    assert_eq!(
        lhs_ty, var_ty,
        "mismatched input/output types ({lhs_ty}, {var_ty}) for {op}"
    );
    assert_eq!(var_ty, "i64", "unsupported type {var_ty} for {op}");

    format!(
        "  {} = {op} {var_ty} {}, {}",
        ToQir::<String>::to_qir(&variable.id, program),
        get_value_as_str(lhs, program),
        get_value_as_str(rhs, program)
    )
}

fn phi_to_qir(
    args: &[(rir::Operand, rir::BlockId)],
    variable: rir::Variable,
    program: &rir::Program,
) -> String {
    assert!(
        !args.is_empty(),
        "phi instruction should have at least one argument"
    );
    let var_ty = get_variable_ty(variable);
    let args = args
        .iter()
        .map(|(arg, block_id)| {
            let arg_ty = get_value_ty(arg);
            assert_eq!(
                arg_ty, var_ty,
                "mismatched types ({var_ty} [... {arg_ty}]) for phi"
            );
            format!(
                "[{}, %{}]",
                get_value_as_str(arg, program),
                ToQir::<String>::to_qir(block_id, program)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "  {} = phi {var_ty} {args}",
        ToQir::<String>::to_qir(&variable.id, program)
    )
}

fn get_value_as_str(value: &rir::Operand, program: &rir::Program) -> String {
    match value {
        rir::Operand::Literal(lit) => match lit {
            rir::Literal::Bool(b) => format!("{b}"),
            rir::Literal::Double(d) => {
                if (d.floor() - d.ceil()).abs() < f64::EPSILON {
                    // The value is a whole number, which requires at least one decimal point
                    // to differentiate it from an integer value.
                    format!("{d:.1}")
                } else {
                    format!("{d}")
                }
            }
            rir::Literal::Integer(i) => format!("{i}"),
            rir::Literal::Pointer => "null".to_string(),
            rir::Literal::Qubit(q) => format!("{q}"),
            rir::Literal::Result(r) => format!("{r}"),
        },
        rir::Operand::Variable(var) => ToQir::<String>::to_qir(&var.id, program),
    }
}

fn get_value_ty(lhs: &rir::Operand) -> &str {
    match lhs {
        rir::Operand::Literal(lit) => match lit {
            rir::Literal::Integer(_) => "i64",
            rir::Literal::Bool(_) => "i1",
            rir::Literal::Double(_) => "f64",
            rir::Literal::Qubit(_) => "%Qubit*",
            rir::Literal::Result(_) => "%Result*",
            rir::Literal::Pointer => "i8*",
        },
        rir::Operand::Variable(var) => get_variable_ty(*var),
    }
}

fn get_variable_ty(variable: rir::Variable) -> &'static str {
    match variable.ty {
        rir::Ty::Integer => "i64",
        rir::Ty::Boolean => "i1",
        rir::Ty::Double => "f64",
        rir::Ty::Qubit => "%Qubit*",
        rir::Ty::Result => "%Result*",
        rir::Ty::Pointer => "i8*",
    }
}

impl ToQir<String> for rir::BlockId {
    fn to_qir(&self, _program: &rir::Program) -> String {
        format!("block_{}", self.0)
    }
}

impl ToQir<String> for rir::Block {
    fn to_qir(&self, program: &rir::Program) -> String {
        self.0
            .iter()
            .map(|instr| ToQir::<String>::to_qir(instr, program))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl ToQir<String> for rir::Callable {
    fn to_qir(&self, program: &rir::Program) -> String {
        let input_type = self
            .input_type
            .iter()
            .map(|t| ToQir::<String>::to_qir(t, program))
            .collect::<Vec<_>>()
            .join(", ");
        let output_type = ToQir::<String>::to_qir(&self.output_type, program);
        let Some(entry_id) = self.body else {
            return format!(
                "declare {output_type} @{}({input_type}){}",
                self.name,
                if self.call_type == rir::CallableType::Measurement {
                    // Measurement callables are a special case that needs the irreversable attribute.
                    " #1"
                } else {
                    ""
                }
            );
        };
        let mut body = String::new();
        let mut all_blocks = vec![entry_id];
        all_blocks.extend(get_all_block_successors(entry_id, program));
        for block_id in all_blocks {
            let block = program.get_block(block_id);
            body.push_str(&format!(
                "{}:\n{}\n",
                ToQir::<String>::to_qir(&block_id, program),
                ToQir::<String>::to_qir(block, program)
            ));
        }
        assert!(
            input_type.is_empty(),
            "entry point should not have an input"
        );
        format!("define {output_type} @ENTRYPOINT__main() #0 {{\n{body}}}",)
    }
}

impl ToQir<String> for rir::Program {
    fn to_qir(&self, _program: &rir::Program) -> String {
        let callables = self
            .callables
            .iter()
            .map(|(_, callable)| ToQir::<String>::to_qir(callable, self))
            .collect::<Vec<_>>()
            .join("\n\n");
        let profile = if self.config.is_base() {
            "base_profile"
        } else {
            "adaptive_profile"
        };
        let body = format!(
            include_str!("./qir/template.ll"),
            callables, profile, self.num_qubits, self.num_results
        );
        let flags = get_module_metadata(self);
        body + "\n" + &flags
    }
}

/// Create the module metadata for the given program.
/// creating the `llvm.module.flags` and its associated values.
fn get_module_metadata(program: &rir::Program) -> String {
    let mut flags = String::new();

    // push the default attrs, we don't have any config values
    // for now that would change any of them.
    flags.push_str(
        r#"
!0 = !{i32 1, !"qir_major_version", i32 1}
!1 = !{i32 7, !"qir_minor_version", i32 0}
!2 = !{i32 1, !"dynamic_qubit_management", i1 false}
!3 = !{i32 1, !"dynamic_result_management", i1 false}
"#,
    );

    let mut index = 4;

    // If we are not in the base profile, we need to add the capabilities
    // associated with the adaptive profile.
    if !program.config.is_base() {
        // loop through the capabilities and add them to the metadata
        // for values that we can generate.
        for cap in program.config.capabilities.iter() {
            let name = match cap {
                TargetCapabilityFlags::QubitReset => "qubit_resetting",
                TargetCapabilityFlags::IntegerComputations => "classical_ints",
                TargetCapabilityFlags::FloatingPointComputations => "classical_floats",
                TargetCapabilityFlags::BackwardsBranching => "backwards_branching",
                _ => continue,
            };
            flags.push_str(&format!(
                "!{} = !{{i32 {}, !\"{}\", i1 {}}}\n",
                index, 1, name, true
            ));
            index += 1;
        }

        // loop through the capabilities that are missing and add them to the metadata
        // as not supported.
        let missing = TargetCapabilityFlags::all().difference(program.config.capabilities);
        for cap in missing.iter() {
            let name = match cap {
                TargetCapabilityFlags::QubitReset => "qubit_resetting",
                TargetCapabilityFlags::IntegerComputations => "classical_ints",
                TargetCapabilityFlags::FloatingPointComputations => "classical_floats",
                TargetCapabilityFlags::BackwardsBranching => "backwards_branching",
                _ => continue,
            };
            flags.push_str(&format!(
                "!{} = !{{i32 {}, !\"{}\", i1 {}}}\n",
                index, 1, name, false
            ));
            index += 1;
        }

        // Add the remaining extension capabilities as not supported.
        // We can't generate these values yet so we just add them as false.
        let unmapped_capabilities = [
            "classical_fixed_points",
            "user_functions",
            "multiple_target_branching",
        ];
        for capability in unmapped_capabilities {
            flags.push_str(&format!(
                "!{} = !{{i32 {}, !\"{}\", i1 {}}}\n",
                index, 1, capability, false
            ));
            index += 1;
        }
    }

    let mut metadata_def = String::new();
    metadata_def.push_str("!llvm.module.flags = !{");
    for i in 0..index - 1 {
        metadata_def.push_str(&format!("!{i}, "));
    }
    metadata_def.push_str(&format!("!{}}}\n", index - 1));
    metadata_def + &flags
}
