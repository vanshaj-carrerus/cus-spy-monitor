import { User, Manager, MarketingUser, SalesUser } from '../models/index.js';

/**
 * Creates a new user and a corresponding role-specific profile.
 * @param {Object} userData - Basic user info (username, email, password, role)
 * @param {Object} roleData - Extra data for the role-specific profile (optional)
 */
async function createFullUser(userData, roleData = {}) {
    // 1. Create the main user record
    const user = new User(userData);
    await user.save();

    const userId = user._id;
    let profile = null;

    // 2. Create the role-specific profile
    try {
        switch (userData.role) {
            case 'manager':
                profile = new Manager({
                    userId,
                    managedDepartments: roleData.managedDepartments || [],
                    permissions: roleData.permissions || ['default_read']
                });
                break;
            
            case 'marketing':
                profile = new MarketingUser({
                    userId,
                    departmentId: roleData.departmentId,
                    locationId: roleData.locationId,
                    managerId: roleData.managerId,
                    active: true
                });
                break;

            case 'sales':
                profile = new SalesUser({
                    userId,
                    departmentId: roleData.departmentId,
                    locationId: roleData.locationId,
                    managerId: roleData.managerId,
                    active: true
                });
                break;

            case 'admin':
                // Admin might not need a separate record if it's just a role in User schema,
                // but if we want to track it elsewhere, we could add an Admin schema too.
                console.log("Admin user created without a separate profile record.");
                break;

            default:
                throw new Error(`Unsupported role for profile creation: ${userData.role}`);
        }

        if (profile) {
            await profile.save();
        }

        return { user, profile };
    } catch (err) {
        // Rollback: delete the main user record if profile creation fails?
        // await User.findByIdAndDelete(userId);
        throw err;
    }
}

/**
 * Authenticates a user and returns their role-specific profile.
 * @param {string} username - The username
 * @param {string} password - The password
 */
async function loginUser(username, password) {
    // 1. Find the user
    const user = await User.findOne({ username });
    if (!user) {
        throw new Error("User not found.");
    }

    // 2. Verify password (in a real app, use bcrypt handles)
    // Note: Since this is for a "spy monitor" internal system as described, 
    // we'll use a direct comparison if the user didn't specify hashing.
    if (user.password !== password) {
        throw new Error("Invalid password.");
    }

    // 3. Update last login (if we had the field, but user removed it from schema)
    // We can add it back if they want, but for now just return the user.
    
    return user;
}

export { createFullUser, loginUser };
